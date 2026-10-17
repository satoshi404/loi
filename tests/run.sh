#!/bin/bash
cd "$(dirname "$0")/.."
total=0
pass=0
fail=0

# Define expected exit codes for specific test files
get_expected() {
  case "$1" in
    tests/01_effects_ok.loi) echo 7 ;;
    tests/04_metaprogramming.loi) echo 43 ;;
    tests/05_reflection.loi) echo 53 ;;
    tests/06_struct_layout.loi) echo 21 ;;
    tests/07_regions.loi) echo 42 ;;
    tests/08_include_guard.loi) echo 42 ;;
    tests/13_macros_trio.loi) echo 77 ;;
    tests/14_fn_pointers.loi) echo 1 ;;
    tests/15_arrays_postfix.loi) echo 10 ;;
    tests/18_conditionals.loi) echo 20 ;;
    tests/19_effects_callbacks_ok.loi) echo 33 ;;
    tests/20_macros_variadic.loi) echo 68 ;;
    tests/21_include_guard.loi) echo 1 ;;
    tests/26_inc_forms.loi) echo 42 ;;
    tests/29_std_package.loi) echo 42 ;;
    tests/31_package_dotted.loi) echo 42 ;;
    tests/32_attr_link_name.loi) echo 42 ;;
    tests/33_attr_visibility.loi) echo 42 ;;
    tests/35_attr_flags.loi) echo 42 ;;
    tests/36_package_namespace.loi) echo 42 ;;
    tests/39_local_var_visibility.loi) echo 42 ;;
    tests/40_local_var_shadow.loi) echo 42 ;;
    tests/41_must_use_warn.loi) echo 42 ;;
    tests/42_must_use_used.loi) echo 42 ;;
    tests/43_cap_basic.loi) echo 42 ;;
    tests/44_cap_refine.loi) echo 42 ;;
    tests/45_cap_needs.loi) echo 42 ;;
    tests/46_cap_effects_bridge.loi) echo 42 ;;
    tests/47_cap_region.loi) echo 42 ;;
    *) echo "none" ;;
  esac
}

run_test() {
  f="$1"
  total=$((total+1))
  out=$(./loi "$f" --run 2>&1)
  rc=$?
  rc=$((rc & 0xFF))
  bname=$(basename "$f")

  if [[ "$bname" == *"fail"* || "$bname" == "02_effects_violation.loi" || "$bname" == "03_effects_assign_drop.loi" || "$bname" == "09_parse_error.loi" || "$bname" == "10_type_error.loi" || "$bname" == "11_preproc_error.loi" || "$bname" == "12_include_removed.loi" || "$bname" == "16_prefix_array_removed.loi" || "$bname" == "17_effects_fnptr_drop.loi" || "$bname" == "27_import_removed.loi" ]]; then
    if [ "$rc" -ne 0 ]; then
      pass=$((pass+1))
    else
      echo "FAIL (should fail but rc=0): $f"
      fail=$((fail+1))
    fi
  else
    exp=$(get_expected "$f")
    if [ "$exp" = "none" ]; then
      if [ "$rc" -eq 0 ]; then
        pass=$((pass+1))
      else
        echo "FAIL (rc=$rc): $f"
        fail=$((fail+1))
      fi
    else
      if [ "$rc" -eq "$exp" ]; then
        pass=$((pass+1))
      else
        echo "FAIL (rc=$rc, expected=$exp): $f"
        fail=$((fail+1))
      fi
    fi
  fi
}

for f in tests/*.loi; do
  run_test "$f"
done

# @attr(must_use) must warn on a discarded result but stay advisory.
total=$((total+1))
mu_stderr=$(./loi tests/41_must_use_warn.loi --run 2>&1 >/dev/null)
if echo "$mu_stderr" | grep -q "must_use"; then
  pass=$((pass+1))
else
  echo "FAIL (expected a must_use warning): tests/41_must_use_warn.loi"
  fail=$((fail+1))
fi

total=$((total+1))
mu_used=$(./loi tests/42_must_use_used.loi --run 2>&1 >/dev/null)
if echo "$mu_used" | grep -q "must_use"; then
  echo "FAIL (unexpected must_use warning): tests/42_must_use_used.loi"
  fail=$((fail+1))
else
  pass=$((pass+1))
fi

echo "$total tests, $pass passed, $fail failed"
if [ "$fail" -ne 0 ]; then
  exit 1
fi
exit 0
