#!/usr/bin/env python3
"""Generate Loi bindings from a C header.

The generator asks clang for a JSON AST, maps the declarations that live in
the bound header(s) to Loi, and writes:

* a vendored ``.loi`` file with ``opaque`` handles, ``shape`` definitions,
  integer constants and ``extern flow`` declarations;
* optionally a C shim (``--shim``) for calls whose C types cannot be passed
  directly from Loi (structs by value, struct pointers, out parameters);
* optionally a ``skipped.txt`` report explaining every declaration that was
  not bound.

The output is intentionally conservative: anything the generator cannot map
without guessing is skipped and reported.  In particular, callbacks,
variadics, unions, bitfields, inline arrays, pointer-to-pointer parameters,
non-const ``char *`` buffers and pointers to unsafe records are not bound.

Usage:

    python3 tools/bindgen/bindgen.py \
        --header include/SDL.h \
        --output vendor/sdl2/sdl2.loi \
        --shim vendor/sdl2/sdl2_shim.c \
        --skipped vendor/sdl2/skipped.txt \
        --module sdl2 \
        --prefix SDL_

The generated shim includes the original header, so the final build needs the
same include path and library flags as the binding:

    loi build game.loi -o game \
        --c-source vendor/sdl2/sdl2_shim.c \
        --cc-flag -Ivendor/sdl2/include \
        --cc-flag -lSDL2

Requires ``clang`` with JSON AST support and Python 3.10+.
"""

from __future__ import annotations

import argparse
import json
import math
import os
import re
import subprocess
import sys
import tempfile
from dataclasses import dataclass, field
from pathlib import Path
from typing import Iterable, Optional

LOI_KEYWORDS = {
    "alloc", "and", "as", "break", "clone", "comptime", "continue", "else",
    "extern", "false", "flow", "for", "give", "if", "import", "in", "mut",
    "not", "opaque", "open", "or", "protect", "sealed", "shape", "true",
    "while",
}

PRIMITIVES = {
    "char": ("i8", "int8_t"),
    "signed char": ("i8", "int8_t"),
    "unsigned char": ("u8", "uint8_t"),
    "short": ("i16", "int16_t"),
    "short int": ("i16", "int16_t"),
    "signed short": ("i16", "int16_t"),
    "signed short int": ("i16", "int16_t"),
    "unsigned short": ("u16", "uint16_t"),
    "unsigned short int": ("u16", "uint16_t"),
    "int": ("i32", "int32_t"),
    "signed": ("i32", "int32_t"),
    "signed int": ("i32", "int32_t"),
    "unsigned": ("u32", "uint32_t"),
    "unsigned int": ("u32", "uint32_t"),
    "long": ("i64", "int64_t"),
    "long int": ("i64", "int64_t"),
    "signed long": ("i64", "int64_t"),
    "signed long int": ("i64", "int64_t"),
    "unsigned long": ("u64", "uint64_t"),
    "unsigned long int": ("u64", "uint64_t"),
    "long long": ("i64", "int64_t"),
    "long long int": ("i64", "int64_t"),
    "signed long long": ("i64", "int64_t"),
    "signed long long int": ("i64", "int64_t"),
    "unsigned long long": ("u64", "uint64_t"),
    "unsigned long long int": ("u64", "uint64_t"),
    "float": ("f32", "float"),
    "double": ("f64", "double"),
    "bool": ("bool", "bool"),
    "_Bool": ("bool", "bool"),
    "int8_t": ("i8", "int8_t"),
    "uint8_t": ("u8", "uint8_t"),
    "int16_t": ("i16", "int16_t"),
    "uint16_t": ("u16", "uint16_t"),
    "int32_t": ("i32", "int32_t"),
    "uint32_t": ("u32", "uint32_t"),
    "int64_t": ("i64", "int64_t"),
    "uint64_t": ("u64", "uint64_t"),
    "size_t": ("u64", "uint64_t"),
    "ssize_t": ("i64", "int64_t"),
    "intptr_t": ("i64", "int64_t"),
    "uintptr_t": ("u64", "uint64_t"),
    "ptrdiff_t": ("i64", "int64_t"),
}

C_INT_MIN = -(2 ** 31)
C_INT_MAX = 2 ** 31 - 1
C_UINT_MAX = 2 ** 32 - 1
C_LONG_MIN = -(2 ** 63)
C_LONG_MAX = 2 ** 63 - 1
C_ULONG_MAX = 2 ** 64 - 1


def sanitize(name: str, used: set[str], suffix: str = "") -> str:
    """Turn a C identifier or field name into a unique Loi identifier."""
    cleaned = re.sub(r"[^A-Za-z0-9_]", "_", name.strip())
    if not cleaned:
        cleaned = "c"
    if cleaned[0].isdigit():
        cleaned = "_" + cleaned
    if cleaned in LOI_KEYWORDS:
        cleaned += "_"
    if suffix:
        cleaned += suffix
    candidate = cleaned
    index = 2
    while candidate in used:
        candidate = f"{cleaned}_{index}"
        index += 1
    used.add(candidate)
    return candidate


def strip_attributes(text: str) -> str:
    previous = None
    while previous != text:
        previous = text
        text = re.sub(r"__attribute__\s*\(\(.*?\)\)", "", text)
        text = re.sub(r"__declspec\s*\(.*?\)", "", text)
        text = re.sub(r"\[\[.*?\]\]", "", text)
        text = re.sub(r"\b(?:const|volatile|_Atomic|restrict|__restrict|__restrict__|"
                      r"_Nullable|_Nonnull|_Null_unspecified|__ptr32|__ptr64|"
                      r"__unaligned|__w64)\b", "", text)
    text = text.replace(" \t", " ")
    text = re.sub(r"\s+", " ", text).strip()
    text = re.sub(r"\s*\*\s*", " *", text)
    return text


def base_and_pointers(qual_type: str) -> tuple[str, int]:
    """Split ``const Foo * const *`` into its base and top-level star count."""
    text = strip_attributes(qual_type)
    stars = 0
    while True:
        text = text.rstrip()
        text = re.sub(r"\s+(?:const|volatile|restrict|_Atomic)$", "", text).rstrip()
        if text.endswith("*"):
            stars += 1
            text = text[:-1]
            continue
        break
    return text.strip(), stars


def split_function_return(qual_type: str) -> str:
    """Return the return-type prefix of a C function type string."""
    depth = 0
    for index, char in enumerate(qual_type):
        if char == "[":
            depth += 1
        elif char == "]":
            depth -= 1
        elif char == "(" and depth == 0:
            return qual_type[:index].strip()
    return qual_type.strip()


def loc_file(loc: dict | None) -> Optional[str]:
    """Return the source file recorded in a clang location, if any."""
    current = loc
    depth = 0
    while isinstance(current, dict) and depth < 8:
        file = current.get("file")
        if file:
            return file
        current = current.get("includedFrom")
        depth += 1
    return None


def loc_is_bound(loc: dict | None, bind_dirs: list[Path]) -> bool:
    """True when a declaration comes from the bound header tree."""
    if not loc:
        return False
    file = loc_file(loc)
    if file:
        resolved = Path(file)
        if not resolved.is_absolute():
            resolved = Path.cwd() / resolved
        resolved = resolved.resolve()
        return any(_is_within(resolved, directory) for directory in bind_dirs)
    # clang omits `file` when it is the same as the previous declaration's
    # file.  Direct includes of the wrapper therefore have only includedFrom.
    previous = loc.get("includedFrom")
    while isinstance(previous, dict):
        file = previous.get("file")
        if file:
            resolved = Path(file)
            if not resolved.is_absolute():
                resolved = Path.cwd() / resolved
            resolved = resolved.resolve()
            return any(_is_within(resolved, directory) for directory in bind_dirs)
        previous = previous.get("includedFrom")
    return False


def _is_within(path: Path, directory: Path) -> bool:
    try:
        path.relative_to(directory.resolve())
        return True
    except ValueError:
        return False


@dataclass
class FieldDecl:
    name: str
    qual_type: str
    bitfield: bool


@dataclass
class Record:
    id: str
    tag: str
    union: bool
    complete: bool
    c_name: str = ""
    loi_name: Optional[str] = None
    fields: list[FieldDecl] = field(default_factory=list)
    attrs: str = ""
    shape_fields: Optional[list[tuple[str, "Mapped"]]] = None
    shape_state: int = 0
    unsafe_reason: Optional[str] = None


@dataclass
class Enum:
    id: str
    tag: str
    c_name: str = ""
    constants: list[tuple[str, int]] = field(default_factory=list)


@dataclass
class Typedef:
    name: str
    target: str
    record_id: Optional[str] = None
    enum_id: Optional[str] = None


@dataclass
class Function:
    name: str
    qual_type: str
    params: list[dict]
    variadic: bool
    storage: Optional[str]
    inline: bool


@dataclass
class Mapped:
    kind: str
    loi: str = ""
    c: str = ""
    c_orig: str = ""
    record: Optional[Record] = None
    shape: bool = False


@dataclass
class BoundParam:
    name: str
    direction: str
    mapped: Mapped
    needs_shim: bool = False
    const: bool = False
    pointer: bool = False


@dataclass
class BoundFunction:
    name: str
    loi_name: str
    link_name: Optional[str]
    params: list[BoundParam]
    result: Mapped
    needs_shim: bool
    shim_symbol: Optional[str] = None


class Bindgen:
    def __init__(
        self,
        bind_dirs: list[Path],
        prefixes: list[str],
        module: str,
    ) -> None:
        self.bind_dirs = bind_dirs
        self.prefixes = prefixes
        self.module = module
        self.typedefs: dict[str, Typedef] = {}
        self.records: dict[str, Record] = {}
        self.records_by_tag: dict[str, Record] = {}
        self.enums: dict[str, Enum] = {}
        self.enums_by_tag: dict[str, Enum] = {}
        self.enum_constants: dict[str, int] = {}
        self.functions: list[Function] = []
        self.used_names: set[str] = set()
        self.used_shapes: list[Record] = []
        self.used_opaque: dict[str, Record | None] = {}
        self.constants: list[tuple[str, str, int]] = []
        self.skips: list[str] = []
        self.void_ptr_name: Optional[str] = None

    # ------------------------------------------------------------------
    # AST collection
    # ------------------------------------------------------------------

    def collect(self, node: dict) -> None:
        kind = node.get("kind")
        if kind == "TypedefDecl":
            self._collect_typedef(node)
        elif kind == "RecordDecl":
            self._collect_record(node)
        elif kind == "EnumDecl":
            self._collect_enum(node)
        elif kind == "FunctionDecl":
            self._collect_function(node)
        for child in node.get("inner", []):
            self.collect(child)

    def _is_bound(self, node: dict) -> bool:
        return loc_is_bound(node.get("loc"), self.bind_dirs)

    def _collect_typedef(self, node: dict) -> None:
        if not self._is_bound(node):
            return
        name = node.get("name") or ""
        if not name:
            return
        target = node.get("type", {}).get("qualType", "")
        record_id = None
        enum_id = None
        for child in _walk_nodes(node):
            if child.get("kind") == "RecordDecl" and record_id is None:
                record_id = child.get("id")
            elif child.get("kind") == "EnumDecl" and enum_id is None:
                enum_id = child.get("id")
            owned = child.get("ownedTagDecl")
            if isinstance(owned, dict):
                if owned.get("kind") == "RecordDecl" and record_id is None:
                    record_id = owned.get("id")
                elif owned.get("kind") == "EnumDecl" and enum_id is None:
                    enum_id = owned.get("id")
        self.typedefs[name] = Typedef(name, target, record_id, enum_id)

    def _collect_record(self, node: dict) -> None:
        if not self._is_bound(node):
            return
        record_id = node.get("id")
        if not record_id:
            return
        existing = self.records.get(record_id)
        complete = bool(node.get("completeDefinition"))
        if existing and existing.complete and not complete:
            return
        tag = node.get("name") or ""
        prior = self.records_by_tag.get(tag) if tag else None
        if prior is not None and prior.id != record_id:
            # A forward declaration and its definition may use separate ids.
            # Keep one Record object so typedef links and fields agree.
            if prior.complete and not complete:
                self.records[record_id] = prior
                return
            if complete and not prior.complete:
                record = prior
                record.complete = True
                record.tag = tag or record.tag
                record.union = node.get("tagUsed") == "union"
                record.attrs = " ".join(
                    str(attr.get("kind", "")) for attr in node.get("attrs", [])
                )
                record.fields = [
                    FieldDecl(
                        name=child.get("name") or "",
                        qual_type=child.get("type", {}).get("qualType", ""),
                        bitfield=bool(child.get("isBitfield")),
                    )
                    for child in node.get("inner", [])
                    if child.get("kind") == "FieldDecl"
                ]
                self.records[record_id] = record
                self.records_by_tag[tag] = record
                return
        record = existing or Record(
            id=record_id,
            tag=node.get("name") or "",
            union=node.get("tagUsed") == "union",
            complete=False,
        )
        record.tag = node.get("name") or record.tag
        record.union = node.get("tagUsed") == "union"
        record.complete = complete
        attrs = " ".join(
            str(attr.get("kind", "")) for attr in node.get("attrs", [])
        )
        record.attrs = attrs
        if complete:
            fields: list[FieldDecl] = []
            for child in node.get("inner", []):
                if child.get("kind") != "FieldDecl":
                    continue
                qual = child.get("type", {}).get("qualType", "")
                fields.append(
                    FieldDecl(
                        name=child.get("name") or "",
                        qual_type=qual,
                        bitfield=bool(child.get("isBitfield")),
                    )
                )
            record.fields = fields
        self.records[record_id] = record
        if record.tag:
            self.records_by_tag[record.tag] = record
        if not record.c_name:
            record.c_name = f"struct {record.tag}" if record.tag else ""

    def _collect_enum(self, node: dict) -> None:
        if not self._is_bound(node):
            return
        enum_id = node.get("id")
        if not enum_id:
            return
        tag = node.get("name") or ""
        enum = self.enums.get(enum_id) or Enum(enum_id, tag)
        enum.tag = tag or enum.tag
        constants: list[tuple[str, int]] = []
        next_value = 0
        for child in node.get("inner", []):
            if child.get("kind") != "EnumConstantDecl":
                continue
            name = child.get("name") or ""
            value = _enum_value(child, next_value)
            if value is None:
                continue
            constants.append((name, value))
            next_value = value + 1
            self.enum_constants[name] = value
        enum.constants = constants
        self.enums[enum_id] = enum
        if tag:
            self.enums_by_tag[tag] = enum
        if not enum.c_name:
            enum.c_name = f"enum {tag}" if tag else ""

    def _collect_function(self, node: dict) -> None:
        if not self._is_bound(node):
            return
        name = node.get("name") or ""
        if not name:
            return
        params = [child for child in node.get("inner", [])
                  if child.get("kind") == "ParmVarDecl"]
        function = Function(
            name=name,
            qual_type=node.get("type", {}).get("qualType", ""),
            params=params,
            variadic=bool(node.get("variadic")),
            storage=node.get("storageClass"),
            inline=bool(node.get("inline")),
        )
        # Keep one declaration per C name, preferring one with parameters.
        for index, existing in enumerate(self.functions):
            if existing.name == name:
                if len(function.params) > len(existing.params):
                    self.functions[index] = function
                return
        self.functions.append(function)

    # ------------------------------------------------------------------
    # Naming
    # ------------------------------------------------------------------

    def assign_names(self) -> None:
        # Link typedef aliases to their records and enums.
        for typedef in self.typedefs.values():
            if typedef.record_id and typedef.record_id in self.records:
                record = self.records[typedef.record_id]
                if not record.c_name:
                    record.c_name = typedef.name
            if typedef.enum_id and typedef.enum_id in self.enums:
                enum = self.enums[typedef.enum_id]
                if not enum.c_name:
                    enum.c_name = typedef.name

        # Prefer the first public typedef name, then the struct tag.
        alias_for_record: dict[str, str] = {}
        for typedef in self.typedefs.values():
            if typedef.record_id and typedef.record_id not in alias_for_record:
                alias_for_record[typedef.record_id] = typedef.name
        for record in self.records.values():
            preferred = alias_for_record.get(record.id) or record.tag
            if not preferred:
                continue
            record.loi_name = sanitize(preferred, self.used_names)
            if not record.c_name:
                record.c_name = preferred

        alias_for_enum: dict[str, str] = {}
        for typedef in self.typedefs.values():
            if typedef.enum_id and typedef.enum_id not in alias_for_enum:
                alias_for_enum[typedef.enum_id] = typedef.name
        for enum in self.enums.values():
            enum.c_name = alias_for_enum.get(enum.id) or enum.c_name or enum.tag

    def opaque_for(self, record: Record) -> Optional[Mapped]:
        """Return (and register) an opaque handle for an incomplete/unsafe record."""
        name = record.loi_name
        if not name:
            return None
        self.used_opaque[name] = record
        return Mapped("opaque", name, "void *", record=record)

    def void_ptr(self) -> Mapped:
        if self.void_ptr_name is None:
            self.void_ptr_name = sanitize("VoidPtr", self.used_names)
            self.used_opaque[self.void_ptr_name] = None
        return Mapped("opaque", self.void_ptr_name, "void *")

    def mark_shape_used(self, record: Record) -> None:
        if record.shape_state == 2 and record not in self.used_shapes:
            self.used_shapes.append(record)

    # ------------------------------------------------------------------
    # Type mapping
    # ------------------------------------------------------------------

    def resolve_base(self, base: str) -> tuple[str, object]:
        """Resolve a base type name through typedefs and struct tags."""
        seen: set[str] = set()
        current = base
        while True:
            if current in seen:
                return "unknown", None
            seen.add(current)
            if current.startswith("struct "):
                tag = current[len("struct "):].strip()
                record = self.records_by_tag.get(tag)
                return ("record", record) if record else ("unknown", None)
            if current.startswith("union "):
                tag = current[len("union "):].strip()
                record = self.records_by_tag.get(tag)
                return ("record", record) if record else ("unknown", None)
            if current.startswith("enum "):
                tag = current[len("enum "):].strip()
                enum = self.enums_by_tag.get(tag)
                return ("enum", enum) if enum else ("unknown", None)
            typedef = self.typedefs.get(current)
            if typedef:
                if typedef.record_id and typedef.record_id in self.records:
                    return "record", self.records[typedef.record_id]
                if typedef.enum_id and typedef.enum_id in self.enums:
                    return "enum", self.enums[typedef.enum_id]
                current = strip_attributes(typedef.target)
                continue
            if current in PRIMITIVES:
                return "scalar", PRIMITIVES[current]
            return "unknown", None


    def shape_fields(self, record: Record) -> Optional[list[tuple[str, "Mapped"]]]:
        if record.shape_state == 2:
            return record.shape_fields
        if record.shape_state == 1:
            record.unsafe_reason = "contains itself by value"
            return None
        record.shape_state = 1
        if not record.complete:
            record.unsafe_reason = "incomplete record"
            record.shape_state = 0
            return None
        if record.union:
            record.unsafe_reason = "union"
            record.shape_state = 0
            return None
        if not record.fields:
            record.unsafe_reason = "empty record"
            record.shape_state = 0
            return None
        if "PackedAttr" in record.attrs or "AlignedAttr" in record.attrs:
            record.unsafe_reason = "packed or explicitly aligned record"
            record.shape_state = 0
            return None
        fields: list[tuple[str, Mapped]] = []
        used: set[str] = set()
        for declared in record.fields:
            if declared.bitfield:
                record.unsafe_reason = f"bitfield field '{declared.name}'"
                record.shape_state = 0
                return None
            if not declared.name:
                record.unsafe_reason = "anonymous field"
                record.shape_state = 0
                return None
            mapped = self.map_field_type(declared.qual_type)
            if mapped is None:
                record.unsafe_reason = (
                    f"field '{declared.name}' has unsupported type "
                    f"'{declared.qual_type}'"
                )
                record.shape_state = 0
                return None
            fields.append((sanitize(declared.name, used), mapped))
        record.shape_fields = fields
        record.shape_state = 2
        for _, mapped in fields:
            if mapped.kind == "shape" and mapped.record is not None:
                self.mark_shape_used(mapped.record)
        return fields

    def expand_pointer_alias(self, qual_type: str) -> str:
        """Expand typedef aliases such as ``typedef char *String``."""
        current = qual_type
        seen: set[str] = set()
        while True:
            base, pointers = base_and_pointers(current)
            if pointers:
                return current
            if base in seen:
                return current
            typedef = self.typedefs.get(base)
            if typedef is None:
                return current
            seen.add(base)
            target = typedef.target
            _, target_pointers = base_and_pointers(target)
            if target_pointers == 0 and "[" not in target:
                return current
            current = target

    def map_field_type(self, qual_type: str) -> Optional[Mapped]:
        qual_type = self.expand_pointer_alias(qual_type)
        base, pointers = base_and_pointers(qual_type)
        if "(*" in strip_attributes(qual_type) or "(*" in qual_type:
            return None
        if pointers:
            if pointers > 1:
                return None
            if base == "char" and "const" in qual_type:
                return Mapped("cstr", "cstr", "const char *")
            if base == "void":
                return self.void_ptr()
            return None
        if base == "void":
            return None
        kind, value = self.resolve_base(base)
        if kind == "scalar":
            loi, c = value  # type: ignore[misc]
            return Mapped("scalar", loi, c)
        if kind == "enum":
            return Mapped("enum", "i32", "int32_t")
        if kind == "record":
            record: Record = value  # type: ignore[assignment]
            if self.shape_fields(record) is None:
                return None
            return Mapped(
                "shape",
                record.loi_name or "",
                f"loi_t_{record.loi_name}",
                record=record,
                shape=True,
            )
        return None

    # ------------------------------------------------------------------
    # Public function mapping
    # ------------------------------------------------------------------

    def bind_functions(self) -> list[BoundFunction]:
        bound: list[BoundFunction] = []
        for function in sorted(self.functions, key=lambda fn: fn.name):
            if not self._function_allowed(function):
                continue
            mapped = self._map_function(function)
            if mapped is not None:
                bound.append(mapped)
        return bound

    def _function_allowed(self, function: Function) -> bool:
        if function.name in {"main", "SDL_main", "wmain"}:
            self.skips.append(f"{function.name}: entry point")
            return False
        if function.storage == "static" or function.inline:
            self.skips.append(f"{function.name}: static or inline function")
            return False
        if self.prefixes and not any(
            function.name.startswith(prefix) for prefix in self.prefixes
        ):
            return False
        return True

    def _map_function(self, function: Function) -> Optional[BoundFunction]:
        if function.variadic:
            self.skips.append(f"{function.name}: variadic function")
            return None

        result = self.map_return(
            split_function_return(function.qual_type), function.name
        )
        if result is None:
            return None

        params: list[BoundParam] = []
        needs_shim = result.kind == "shape"
        used_params: set[str] = set()
        for index, param in enumerate(function.params):
            qual = param.get("type", {}).get("qualType", "")
            raw_name = param.get("name") or f"arg{index}"
            name = sanitize(raw_name, used_params)
            mapped = self.map_param(qual, function.name, raw_name)
            if mapped is None:
                return None
            direction, mapped_type, shim, is_const = mapped
            if mapped_type.kind == "void":
                continue
            if shim:
                needs_shim = True
            pointer = (
                mapped_type.kind == "shape"
                and base_and_pointers(self.expand_pointer_alias(qual))[1] == 1
            )
            params.append(
                BoundParam(
                    name=name,
                    direction=direction,
                    mapped=mapped_type,
                    needs_shim=shim,
                    const=is_const,
                    pointer=pointer,
                )
            )

        loi_name = self._unique_function_name(function.name)
        link_name = None
        shim_symbol = None
        if needs_shim:
            shim_symbol = sanitize(
                f"loi_bind_{self.module}_{function.name}",
                self.used_names,
            )
            link_name = shim_symbol
        elif loi_name != function.name:
            link_name = function.name

        return BoundFunction(
            name=function.name,
            loi_name=loi_name,
            link_name=link_name,
            params=params,
            result=result,
            needs_shim=needs_shim,
            shim_symbol=shim_symbol,
        )

    def _unique_function_name(self, c_name: str) -> str:
        if c_name not in self.used_names:
            self.used_names.add(c_name)
            return c_name
        return sanitize(c_name, self.used_names, "_fn")

    def map_param(
        self, qual_type: str, function_name: str, param_name: str
    ) -> Optional[tuple[str, Mapped, bool, bool]]:
        qual_type = self.expand_pointer_alias(qual_type)
        base, pointers = base_and_pointers(qual_type)
        cleaned = strip_attributes(qual_type)
        if "(*" in cleaned or "(*" in qual_type:
            self.skips.append(
                f"{function_name}: parameter '{param_name}' is a function pointer"
            )
            return None
        if "[" in base or "]" in base:
            self.skips.append(
                f"{function_name}: parameter '{param_name}' is an array"
            )
            return None
        if pointers > 1:
            self.skips.append(
                f"{function_name}: parameter '{param_name}' is a pointer-to-pointer"
            )
            return None

        is_const = "const" in qual_type
        if pointers == 1:
            if base == "char":
                if is_const:
                    return ("<-", Mapped("cstr", "cstr", "const char *"), False, True)
                self.skips.append(
                    f"{function_name}: parameter '{param_name}' is a mutable "
                    f"char buffer; use a vendored shim"
                )
                return None
            if base == "void":
                return ("<-", self.void_ptr(), False, False)

            kind, value = self.resolve_base(base)
            if kind == "record":
                record: Record = value  # type: ignore[assignment]
                if self.shape_fields(record) is not None:
                    self.mark_shape_used(record)
                    mapped = Mapped(
                        "shape",
                        record.loi_name or "",
                        f"loi_t_{record.loi_name}",
                        record.c_name,
                        record=record,
                        shape=True,
                    )
                    direction = "<-" if is_const else "<->"
                    return (direction, mapped, True, is_const)
                if record.complete:
                    self.skips.append(
                        f"{function_name}: parameter '{param_name}' points to "
                        f"unsafe record '{record.c_name or record.tag}'"
                    )
                    return None
                mapped = self.opaque_for(record)
                if mapped is None:
                    self.skips.append(
                        f"{function_name}: parameter '{param_name}' points to "
                        f"an unnamed record"
                    )
                    return None
                return ("<-", mapped, False, is_const)
            if kind == "enum":
                enum: Enum = value  # type: ignore[assignment]
                if is_const:
                    self.skips.append(
                        f"{function_name}: parameter '{param_name}' is a const "
                        f"pointer to an enum"
                    )
                    return None
                return (
                    "->",
                    Mapped("out_enum", "i32", "int32_t *", enum.c_name),
                    True,
                    False,
                )
            if kind == "scalar":
                if is_const:
                    self.skips.append(
                        f"{function_name}: parameter '{param_name}' is a const "
                        f"pointer to a scalar"
                    )
                    return None
                loi, c = value  # type: ignore[misc]
                return (
                    "->",
                    Mapped("out_scalar", loi, f"{c} *", c),
                    True,
                    False,
                )
            self.skips.append(
                f"{function_name}: parameter '{param_name}' has unsupported "
                f"pointer type '{qual_type}'"
            )
            return None

        if base == "void":
            return ("<-", Mapped("void"), False, False)
        kind, value = self.resolve_base(base)
        if kind == "scalar":
            loi, c = value  # type: ignore[misc]
            return ("<-", Mapped("scalar", loi, c, c), False, False)
        if kind == "enum":
            return ("<-", Mapped("enum", "i32", "int32_t", "int32_t"), False, False)
        if kind == "record":
            record: Record = value  # type: ignore[assignment]
            if self.shape_fields(record) is not None:
                self.mark_shape_used(record)
                return (
                    "<-",
                    Mapped(
                        "shape",
                        record.loi_name or "",
                        f"loi_t_{record.loi_name}",
                        record.c_name,
                        record=record,
                        shape=True,
                    ),
                    True,
                    False,
                )
            self.skips.append(
                f"{function_name}: parameter '{param_name}' is an unsafe record "
                f"by value ('{record.c_name or record.tag}')"
            )
            return None
        self.skips.append(
            f"{function_name}: parameter '{param_name}' has unsupported type "
            f"'{qual_type}'"
        )
        return None

    def map_return(self, qual_type: str, function_name: str) -> Optional[Mapped]:
        qual_type = self.expand_pointer_alias(qual_type)
        base, pointers = base_and_pointers(qual_type)
        cleaned = strip_attributes(qual_type)
        if "(*" in cleaned or "(*" in qual_type:
            self.skips.append(f"{function_name}: returns a function pointer")
            return None
        if pointers > 1:
            self.skips.append(f"{function_name}: returns a pointer-to-pointer")
            return None
        if pointers == 1:
            if base == "char" and "const" in qual_type:
                return Mapped("cstr", "cstr", "const char *")
            if base == "void":
                return self.void_ptr()
            kind, value = self.resolve_base(base)
            if kind == "record":
                record: Record = value  # type: ignore[assignment]
                if self.shape_fields(record) is None:
                    mapped = self.opaque_for(record)
                    if mapped is None:
                        self.skips.append(
                            f"{function_name}: returns a pointer to an unnamed record"
                        )
                    return mapped
                self.skips.append(
                    f"{function_name}: returns a pointer to record "
                    f"'{record.c_name or record.tag}'"
                )
                return None
            self.skips.append(
                f"{function_name}: returns unsupported pointer type '{qual_type}'"
            )
            return None
        if base == "void":
            return Mapped("void")
        kind, value = self.resolve_base(base)
        if kind == "scalar":
            loi, c = value  # type: ignore[misc]
            return Mapped("scalar", loi, c, c)
        if kind == "enum":
            return Mapped("enum", "i32", "int32_t", "int32_t")
        if kind == "record":
            record: Record = value  # type: ignore[assignment]
            if self.shape_fields(record) is None:
                self.skips.append(
                    f"{function_name}: returns unsafe record "
                    f"'{record.c_name or record.tag}' by value"
                )
                return None
            self.mark_shape_used(record)
            return Mapped(
                "shape",
                record.loi_name or "",
                f"loi_t_{record.loi_name}",
                record.c_name,
                record=record,
                shape=True,
            )
        self.skips.append(
            f"{function_name}: returns unsupported type '{qual_type}'"
        )
        return None

    # ------------------------------------------------------------------
    # Constants
    # ------------------------------------------------------------------

    def add_enum_constants(self) -> None:
        for enum in sorted(self.enums.values(), key=lambda item: item.tag):
            for name, value in enum.constants:
                if name in self.used_names:
                    continue
                loi_name = sanitize(name, self.used_names)
                self.constants.append(
                    (loi_name, constant_type(value), value)
                )

    def add_macros(self, definitions: dict[str, str]) -> None:
        evaluator = MacroEvaluator(definitions, self.enum_constants)
        for name in sorted(definitions):
            if name.startswith("_"):
                # Identifiers beginning with an underscore are reserved for
                # the C implementation; do not expose them in bindings.
                continue
            if self.prefixes and not any(
                name.startswith(prefix) for prefix in self.prefixes
            ):
                continue
            if name in self.used_names:
                self.skips.append(
                    f"macro {name}: name already bound as a declaration"
                )
                continue
            try:
                value, unsigned_hint, long_hint = evaluator.evaluate(name)
            except MacroError as error:
                self.skips.append(f"macro {name}: {error}")
                continue
            if value < C_LONG_MIN or value > C_ULONG_MAX:
                self.skips.append(f"macro {name}: value does not fit in 64 bits")
                continue
            loi_name = sanitize(name, self.used_names)
            self.constants.append(
                (loi_name, constant_type(value, unsigned_hint, long_hint), value)
            )

    # ------------------------------------------------------------------
    # Loi emission
    # ------------------------------------------------------------------

    def emit_loi(self, source_header: str, command: str) -> str:
        lines: list[str] = []
        lines.append("// Generated by tools/bindgen/bindgen.py. DO NOT EDIT.")
        lines.append(f"// Source header: {source_header}")
        lines.append(f"// Command: {command}")
        lines.append("//")
        lines.append("// Bindings are intentionally conservative.  Calls that need")
        lines.append("// layout conversion use the accompanying C shim; compile it with")
        lines.append("// `loi build ... --c-source <shim>.c`.")
        lines.append("")

        opaque_names = sorted(self.used_opaque)
        for name in opaque_names:
            lines.append(f"opaque {name}")
        if opaque_names:
            lines.append("")

        for record in self.used_shapes:
            fields = record.shape_fields or []
            lines.append(f"shape {record.loi_name} [")
            for field_name, mapped in fields:
                lines.append(f"    {field_name}: {mapped.loi}")
            lines.append("]")
            lines.append("")

        if self.constants:
            for name, ty, value in self.constants:
                lines.append(f"{name}: {ty} <- {value}")
            lines.append("")

        for function in self.bound_functions:
            header = f"extern flow {function.loi_name}"
            if function.link_name and function.link_name != function.loi_name:
                header += f" as {function.link_name}"
            lines.append(header)
            for param in function.params:
                lines.append(
                    f"    {param.direction} {param.name}: {param.mapped.loi}"
                )
            if function.result.kind != "void":
                lines.append(f"    => {function.result.loi}")
            lines.append("")

        return "\n".join(lines).rstrip() + "\n"

    # ------------------------------------------------------------------
    # C shim emission
    # ------------------------------------------------------------------

    def emit_shim(
        self, source_header: str, command: str, shim_path: str
    ) -> str:
        lines: list[str] = []
        lines.append("/* Generated by tools/bindgen/bindgen.py. DO NOT EDIT. */")
        lines.append(f"/* Source header: {source_header} */")
        lines.append(f"/* Command: {command} */")
        lines.append("")
        lines.append("#include <stddef.h>")
        lines.append("#include <stdint.h>")
        lines.append("#include <stdbool.h>")
        lines.append("#include <string.h>")
        lines.append("")
        try:
            relative = os.path.relpath(source_header, Path(shim_path).parent)
        except ValueError:
            relative = source_header
        lines.append(f"#include \"{Path(relative).as_posix()}\"")
        lines.append("")

        for record in self.used_shapes:
            c_name = record.c_name or (
                f"struct {record.tag}" if record.tag else ""
            )
            if not c_name:
                continue
            mirror = f"loi_t_{record.loi_name}"
            lines.append(f"typedef struct {mirror} {mirror};")
            lines.append(f"struct {mirror} {{")
            for field_name, mapped in record.shape_fields or []:
                lines.append(f"    {mapped.c} loi_f_{field_name};")
            lines.append("};")
            lines.append(
                f"_Static_assert(sizeof({mirror}) == sizeof({c_name}), "
                f"\"bindgen: layout mismatch for {record.loi_name}\");"
            )
            lines.append(
                f"_Static_assert(_Alignof({mirror}) == _Alignof({c_name}), "
                f"\"bindgen: alignment mismatch for {record.loi_name}\");"
            )
            lines.append("")

        for function in self.bound_functions:
            if not function.needs_shim:
                continue
            lines.extend(self._emit_shim_function(function))
            lines.append("")

        return "\n".join(lines).rstrip() + "\n"

    def _emit_shim_function(self, function: BoundFunction) -> list[str]:
        symbol = function.shim_symbol or function.name
        params = ", ".join(
            f"{shim_param_type(param)} {param.name}" for param in function.params
        )
        if not params:
            params = "void"
        ret = shim_return_type(function.result)
        lines = [f"{ret} {symbol}({params}) {{"]

        # C locals must not collide with any Loi parameter name (an out
        # parameter may legitimately be called ``result``).
        used_locals: set[str] = {param.name for param in function.params}

        def local(base: str) -> str:
            name = base
            index = 2
            while name in used_locals:
                name = f"{base}_{index}"
                index += 1
            used_locals.add(name)
            return name

        args: list[str] = []
        pre: list[str] = []
        post: list[str] = []
        for param in function.params:
            mapped = param.mapped
            if mapped.kind == "shape" and not param.pointer:
                temp = local(f"c_{param.name}")
                c_type = mapped.c_orig or mapped.c
                pre.append(f"    {c_type} {temp};")
                pre.append(
                    f"    memcpy(&{temp}, &{param.name}, sizeof({temp}));"
                )
                args.append(temp)
            elif mapped.kind == "shape" and param.pointer:
                temp = local(f"c_{param.name}")
                c_type = mapped.c_orig or mapped.c
                pre.append(f"    {c_type} {temp};")
                if param.direction == "<-":
                    # Loi passes a const-pointer shape by value; C wants an
                    # address, so copy the value into a local first.
                    pre.append(
                        f"    memcpy(&{temp}, &{param.name}, sizeof({temp}));"
                    )
                else:
                    pre.append(
                        f"    memcpy(&{temp}, {param.name}, sizeof({temp}));"
                    )
                args.append(f"&{temp}")
                if param.direction == "<->" and not param.const:
                    post.append(
                        f"    memcpy({param.name}, &{temp}, sizeof({temp}));"
                    )
            elif mapped.kind == "out_scalar":
                temp = local(f"c_{param.name}")
                temp_type = mapped.c_orig or "int32_t"
                pre.append(f"    {temp_type} {temp} = 0;")
                args.append(f"&{temp}")
                target = loi_c_type(mapped.loi)
                post.append(f"    *{param.name} = ({target}){temp};")
            elif mapped.kind == "out_enum":
                temp = local(f"c_{param.name}")
                temp_type = mapped.c_orig or "int"
                pre.append(f"    {temp_type} {temp} = ({temp_type})0;")
                args.append(f"&{temp}")
                post.append(f"    *{param.name} = (int32_t){temp};")
            else:
                args.append(param.name)

        lines.extend(pre)
        call = f"{function.name}({', '.join(args)})"
        result_name = local("result")
        if function.result.kind == "void":
            lines.append(f"    {call};")
            lines.extend(post)
        elif function.result.kind == "shape":
            c_result = local("c_result")
            c_type = function.result.c_orig or function.result.c
            lines.append(f"    {c_type} {c_result} = {call};")
            target = f"loi_t_{function.result.loi}"
            lines.append(f"    {target} {result_name};")
            lines.append(
                f"    memcpy(&{result_name}, &{c_result}, sizeof({result_name}));"
            )
            lines.extend(post)
            lines.append(f"    return {result_name};")
        else:
            target = shim_return_type(function.result)
            lines.append(f"    {target} {result_name} = ({target}){call};")
            lines.extend(post)
            lines.append(f"    return {result_name};")
        lines.append("}")
        return lines


def _walk_nodes(node: dict) -> Iterable[dict]:
    yield node
    for child in node.get("inner", []):
        yield from _walk_nodes(child)


def _enum_value(node: dict, fallback: int) -> Optional[int]:
    for child in _walk_nodes(node):
        if child.get("kind") == "ConstantExpr" and "value" in child:
            try:
                return int(child["value"], 0)
            except (TypeError, ValueError):
                pass
        if child.get("kind") == "IntegerLiteral" and "value" in child:
            try:
                return int(child["value"], 0)
            except (TypeError, ValueError):
                pass
        if child.get("kind") == "UnaryOperator" and child.get("opcode") == "-":
            inner = child.get("inner", [])
            if inner and "value" in inner[0]:
                try:
                    return -int(inner[0]["value"], 0)
                except (TypeError, ValueError):
                    pass
    return fallback


def loi_c_type(loi_type: str) -> str:
    return {
        "i8": "int8_t",
        "i16": "int16_t",
        "i32": "int32_t",
        "i64": "int64_t",
        "u8": "uint8_t",
        "u16": "uint16_t",
        "u32": "uint32_t",
        "u64": "uint64_t",
        "f32": "float",
        "f64": "double",
        "bool": "bool",
        "cstr": "const char *",
    }.get(loi_type, "int32_t")


def shim_param_type(param: BoundParam) -> str:
    mapped = param.mapped
    if mapped.kind in ("scalar", "enum"):
        return loi_c_type(mapped.loi)
    if mapped.kind == "cstr":
        return "const char *"
    if mapped.kind == "opaque":
        return "void *"
    if mapped.kind == "shape":
        if param.pointer and param.direction != "<-":
            return f"loi_t_{mapped.loi} *"
        return f"loi_t_{mapped.loi}"
    if mapped.kind == "out_scalar":
        return f"{loi_c_type(mapped.loi)} *"
    if mapped.kind == "out_enum":
        return "int32_t *"
    return "void *"


def shim_return_type(mapped: Mapped) -> str:
    if mapped.kind == "void":
        return "void"
    if mapped.kind in ("scalar", "enum"):
        return loi_c_type(mapped.loi)
    if mapped.kind == "cstr":
        return "const char *"
    if mapped.kind == "opaque":
        return "void *"
    if mapped.kind == "shape":
        return f"loi_t_{mapped.loi}"
    if mapped.kind == "enum":
        return "int32_t"
    return "void"


def constant_type(
    value: int, unsigned_hint: bool = False, long_hint: bool = False
) -> str:
    if long_hint:
        if unsigned_hint:
            return "u64"
        if C_LONG_MIN <= value <= C_LONG_MAX:
            return "i64"
        return "u64"
    if unsigned_hint:
        if value <= C_UINT_MAX:
            return "u32"
        return "u64"
    if value < 0:
        return "i32" if value >= C_INT_MIN else "i64"
    if value <= C_INT_MAX:
        return "i32"
    if value <= C_UINT_MAX:
        return "u32"
    if value <= C_LONG_MAX:
        return "i64"
    return "u64"


class MacroError(Exception):
    pass


TOKEN_RE = re.compile(
    r"\s*(?:(?P<num>0[xX][0-9a-fA-F]+[uUlL]*|0[bB][01]+[uUlL]*|"
    r"[0-9]+[uUlL]*|'(?:\\.|[^'\\])')|"
    r"(?P<op><<|>>|[()|&^~+\-*/%])|"
    r"(?P<id>[A-Za-z_]\w*))"
)


class MacroEvaluator:
    """Evaluate the simple integer constant expressions used by C headers.

    This is deliberately not a C preprocessor.  It handles integer literals,
    parentheses, arithmetic, bit operations, enum constants and object-like
    macros.  Everything else is reported as a skip.
    """

    def __init__(self, definitions: dict[str, str], enum_constants: dict[str, int]):
        self.definitions = definitions
        self.enum_constants = enum_constants
        self.cache: dict[str, tuple[int, bool, bool]] = {}
        self.active: set[str] = set()

    def evaluate(self, name: str) -> tuple[int, bool, bool]:
        if name in self.cache:
            return self.cache[name]
        if name in self.active:
            raise MacroError("recursive macro")
        if name in self.enum_constants:
            return self.enum_constants[name], False, False
        if name not in self.definitions:
            raise MacroError(f"unknown identifier '{name}'")
        text = self.definitions[name].strip()
        if not text:
            raise MacroError("empty macro")
        self.active.add(name)
        try:
            raw_tokens, unsigned_hint, long_hint = self._lex(text)
            tokens: list[tuple] = []
            index = 0
            while index < len(raw_tokens):
                token = raw_tokens[index]
                if token[0] == "id":
                    identifier = token[1]
                    if identifier == "true":
                        tokens.append(("num", 1, False, False))
                        index += 1
                        continue
                    if identifier == "false":
                        tokens.append(("num", 0, False, False))
                        index += 1
                        continue
                    if identifier in self.enum_constants:
                        tokens.append(
                            ("num", self.enum_constants[identifier], False, False)
                        )
                        index += 1
                        continue
                    if (
                        index + 1 < len(raw_tokens)
                        and raw_tokens[index + 1] == ("op", "(")
                    ):
                        raise MacroError(
                            f"function-like macro '{identifier}' is not supported"
                        )
                    child_value, child_unsigned, child_long = self.evaluate(
                        identifier
                    )
                    unsigned_hint = unsigned_hint or child_unsigned
                    long_hint = long_hint or child_long
                    tokens.append(("num", child_value, False, False))
                    index += 1
                    continue
                tokens.append(token)
                index += 1
            parser = IntExprParser(tokens)
            value = parser.parse()
        finally:
            self.active.discard(name)
        if unsigned_hint and value < 0:
            width = 64 if long_hint else 32
            value &= (1 << width) - 1
        result = (value, unsigned_hint, long_hint)
        self.cache[name] = result
        return result

    def _lex(self, text: str) -> tuple[list[tuple], bool, bool]:
        text = re.sub(r"__attribute__\s*\(\(.*?\)\)", " ", text)
        text = re.sub(
            r"\(\s*(?:const\s+)?(?:struct|union|enum)\s+[A-Za-z_]\w*\s*\*?\s*\)",
            " ",
            text,
        )
        text = re.sub(
            r"\(\s*(?:(?:unsigned|signed)\s+)?"
            r"(?:char|short|int|long(?:\s+long)?|float|double|_Bool|bool)"
            r"\s*\*?\s*\)",
            " ",
            text,
        )
        # A single-identifier cast such as (Uint32) or (SDL_bool).
        text = re.sub(
            r"\(\s*[A-Za-z_]\w*\s*\*?\s*\)(?=\s*[A-Za-z0-9_('])",
            " ",
            text,
        )

        tokens: list[tuple] = []
        unsigned_hint = False
        long_hint = False
        position = 0
        while position < len(text):
            match = TOKEN_RE.match(text, position)
            if not match:
                raise MacroError(
                    f"unsupported token near '{text[position:].strip()[:20]}'"
                )
            position = match.end()
            if match.group("num") is not None:
                token = match.group("num")
                if token.startswith("'"):
                    tokens.append(("num", _char_value(token), False, False))
                    continue
                value, is_unsigned, is_long = _parse_int_literal(token)
                unsigned_hint = unsigned_hint or is_unsigned
                long_hint = long_hint or is_long
                tokens.append(("num", value, is_unsigned, is_long))
            elif match.group("op") is not None:
                tokens.append(("op", match.group("op")))
            else:
                tokens.append(("id", match.group("id")))
        if not tokens:
            raise MacroError("empty expression")
        return tokens, unsigned_hint, long_hint


def _parse_int_literal(token: str) -> tuple[int, bool, bool]:
    match = re.match(r"^(.*?)([uUlL]*)$", token)
    if not match:
        raise MacroError(f"bad integer literal '{token}'")
    digits, suffix = match.group(1), match.group(2).lower()
    is_unsigned = "u" in suffix
    is_long = "l" in suffix
    try:
        if re.fullmatch(r"0[0-7]+", digits):
            value = int(digits, 8)
        else:
            value = int(digits, 0)
    except ValueError as error:
        raise MacroError(f"bad integer literal '{token}'") from error
    return value, is_unsigned, is_long


def _char_value(token: str) -> int:
    body = token[1:-1]
    escapes = {
        "n": 10,
        "t": 9,
        "r": 13,
        "0": 0,
        "\\": 92,
        "'": 39,
        '"': 34,
        "a": 7,
        "b": 8,
        "f": 12,
        "v": 11,
    }
    if body.startswith("\\"):
        rest = body[1:]
        if rest.startswith("x"):
            return int(rest[1:], 16)
        if rest in escapes:
            return escapes[rest]
        raise MacroError(f"unsupported character literal {token}")
    if len(body) != 1:
        raise MacroError(f"unsupported character literal {token}")
    return ord(body)


class IntExprParser:
    def __init__(self, tokens: list[tuple]):
        self.tokens = tokens
        self.position = 0

    def parse(self) -> int:
        value = self._bitor()
        if self.position != len(self.tokens):
            token = self.tokens[self.position]
            raise MacroError(f"unsupported operator '{token[1]}'")
        return value

    def _peek(self) -> Optional[str]:
        if self.position >= len(self.tokens):
            return None
        token = self.tokens[self.position]
        return token[1] if token[0] == "op" else None

    def _take(self) -> tuple:
        token = self.tokens[self.position]
        self.position += 1
        return token

    def _expect(self, op: str) -> None:
        if self._peek() != op:
            raise MacroError(f"expected '{op}'")
        self.position += 1

    def _bitor(self) -> int:
        value = self._bitxor()
        while self._peek() == "|":
            self._take()
            value |= self._bitxor()
        return value

    def _bitxor(self) -> int:
        value = self._bitand()
        while self._peek() == "^":
            self._take()
            value ^= self._bitand()
        return value

    def _bitand(self) -> int:
        value = self._shift()
        while self._peek() == "&":
            self._take()
            value &= self._shift()
        return value

    def _shift(self) -> int:
        value = self._add()
        while self._peek() in ("<<", ">>"):
            op = self._take()[1]
            right = self._add()
            if right < 0 or right > 63:
                raise MacroError("shift count out of range")
            if op == "<<":
                value <<= right
            else:
                value >>= right
        return value

    def _add(self) -> int:
        value = self._mul()
        while True:
            op = self._peek()
            if op == "+":
                self._take()
                value += self._mul()
            elif op == "-":
                self._take()
                value -= self._mul()
            else:
                return value

    def _mul(self) -> int:
        value = self._unary()
        while True:
            op = self._peek()
            if op == "*":
                self._take()
                value *= self._unary()
            elif op == "/":
                self._take()
                value = _c_div(value, self._unary())
            elif op == "%":
                self._take()
                value = _c_mod(value, self._unary())
            else:
                return value

    def _unary(self) -> int:
        op = self._peek()
        if op == "+":
            self._take()
            return self._unary()
        if op == "-":
            self._take()
            return -self._unary()
        if op == "~":
            self._take()
            return ~self._unary()
        return self._primary()

    def _primary(self) -> int:
        if self.position >= len(self.tokens):
            raise MacroError("unexpected end of expression")
        token = self._take()
        if token[0] == "num":
            return token[1]
        if token[1] == "(":
            value = self._bitor()
            self._expect(")")
            return value
        raise MacroError(f"unexpected token '{token[1]}'")


def _c_div(left: int, right: int) -> int:
    if right == 0:
        raise MacroError("division by zero")
    quotient = abs(left) // abs(right)
    return quotient if (left < 0) == (right < 0) else -quotient


def _c_mod(left: int, right: int) -> int:
    return left - _c_div(left, right) * right


def collect_macro_definitions(
    clang: str, wrapper: Path, flags: list[str]
) -> dict[str, str]:
    command = [clang, "-dM", "-E", "-x", "c", str(wrapper), *flags]
    result = subprocess.run(command, capture_output=True, text=True)
    if result.returncode != 0:
        raise SystemExit(
            "bindgen: clang could not preprocess the header:\n" + result.stderr
        )
    definitions: dict[str, str] = {}
    for line in result.stdout.splitlines():
        match = re.match(r"#define\s+([A-Za-z_]\w*)(.*)$", line)
        if not match:
            continue
        name, rest = match.group(1), match.group(2)
        if rest.startswith("("):
            continue  # function-like macro
        definitions[name] = rest.strip()
    return definitions


def run_clang_ast(clang: str, wrapper: Path, flags: list[str]) -> dict:
    command = [
        clang,
        "-Xclang",
        "-ast-dump=json",
        "-fsyntax-only",
        "-x",
        "c",
        str(wrapper),
        *flags,
    ]
    result = subprocess.run(command, capture_output=True, text=True)
    if result.returncode != 0:
        raise SystemExit(
            "bindgen: clang could not parse the header:\n" + result.stderr
        )
    try:
        return json.loads(result.stdout)
    except json.JSONDecodeError as error:
        raise SystemExit(f"bindgen: invalid clang JSON: {error}") from error


def main(argv: Optional[list[str]] = None) -> int:
    parser = argparse.ArgumentParser(
        description="Generate Loi bindings from a C header."
    )
    parser.add_argument("--header", required=True, help="C header to bind")
    parser.add_argument("--output", required=True, help="generated .loi file")
    parser.add_argument("--shim", help="generated C shim file")
    parser.add_argument("--skipped", help="write skipped declarations here")
    parser.add_argument(
        "--module",
        default="",
        help="module name used in generated shim symbols (default: header stem)",
    )
    parser.add_argument(
        "--prefix",
        action="append",
        default=[],
        help="only bind functions/macros with this prefix (repeatable)",
    )
    parser.add_argument(
        "--include",
        action="append",
        default=[],
        help="extra include directory for clang and the bound header tree",
    )
    parser.add_argument(
        "--define",
        action="append",
        default=[],
        help="preprocessor definition passed to clang (repeatable)",
    )
    parser.add_argument(
        "--flag",
        action="append",
        default=[],
        help="extra clang flag (repeatable)",
    )
    parser.add_argument(
        "--clang",
        default=os.environ.get("CLANG", "clang"),
        help="clang executable (default: clang or $CLANG)",
    )
    parser.add_argument(
        "--no-macros",
        action="store_true",
        help="do not bind integer macros",
    )
    args = parser.parse_args(argv)

    header = Path(args.header).resolve()
    if not header.is_file():
        print(f"bindgen: header not found: {header}", file=sys.stderr)
        return 2

    include_dirs: list[Path] = []
    for value in args.include:
        directory = Path(value).resolve()
        if directory not in include_dirs:
            include_dirs.append(directory)
    if header.parent not in include_dirs:
        include_dirs.append(header.parent)

    module = args.module or sanitize(
        re.sub(r"\.h$", "", header.name), set()
    )
    flags = [f"-I{directory}" for directory in include_dirs]
    flags.extend(f"-D{value}" for value in args.define)
    flags.extend(args.flag)

    import shlex

    command = " ".join(
        shlex.quote(part)
        for part in [sys.executable, __file__, *sys.argv[1:]]
    )

    with tempfile.TemporaryDirectory(prefix="loi-bindgen-") as temporary:
        temporary_path = Path(temporary)
        wrapper = temporary_path / "bindgen_wrapper.c"
        wrapper.write_text(f'#include "{header}"\n', encoding="utf-8")
        bind_dirs = [*include_dirs, temporary_path]

        ast = run_clang_ast(args.clang, wrapper, flags)
        generator = Bindgen(bind_dirs, args.prefix, module)
        for node in ast.get("inner", []):
            generator.collect(node)
        generator.assign_names()
        generator.bound_functions = generator.bind_functions()
        generator.add_enum_constants()
        if not args.no_macros:
            definitions = collect_macro_definitions(args.clang, wrapper, flags)
            empty_wrapper = temporary_path / "bindgen_empty.c"
            empty_wrapper.write_text("", encoding="utf-8")
            baseline = collect_macro_definitions(
                args.clang, empty_wrapper, flags
            )
            definitions = {
                name: value
                for name, value in definitions.items()
                if baseline.get(name) != value
            }
            generator.add_macros(definitions)

        needs_shim = any(
            function.needs_shim for function in generator.bound_functions
        )
        if needs_shim and not args.shim:
            print(
                "bindgen: this header needs a C shim for safe struct/out-parameter "
                "conversion; pass --shim FILE",
                file=sys.stderr,
            )
            return 2

        output_path = Path(args.output)
        output_path.parent.mkdir(parents=True, exist_ok=True)
        output_path.write_text(
            generator.emit_loi(str(header), command), encoding="utf-8"
        )

        if args.shim:
            shim_path = Path(args.shim)
            shim_path.parent.mkdir(parents=True, exist_ok=True)
            shim_path.write_text(
                generator.emit_shim(str(header), command, str(shim_path)),
                encoding="utf-8",
            )

        if args.skipped:
            skipped_path = Path(args.skipped)
            skipped_path.parent.mkdir(parents=True, exist_ok=True)
            lines = [
                "Declarations skipped by tools/bindgen/bindgen.py",
                f"Header: {header}",
                f"Functions bound: {len(generator.bound_functions)}",
                f"Constants bound: {len(generator.constants)}",
                f"Shapes bound: {len(generator.used_shapes)}",
                f"Opaque handles bound: {len(generator.used_opaque)}",
                "",
            ]
            if generator.skips:
                lines.extend(sorted(generator.skips))
            else:
                lines.append("(none)")
            skipped_path.write_text("\n".join(lines) + "\n", encoding="utf-8")

        print(
            f"bindgen: {len(generator.bound_functions)} flows, "
            f"{len(generator.constants)} constants, "
            f"{len(generator.used_shapes)} shapes, "
            f"{len(generator.used_opaque)} opaque handles, "
            f"{len(generator.skips)} skipped",
            file=sys.stderr,
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
