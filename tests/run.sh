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

echo "$total tests, $pass passed, $fail failed"
if [ "$fail" -ne 0 ]; then
  exit 1
fi
exit 0
