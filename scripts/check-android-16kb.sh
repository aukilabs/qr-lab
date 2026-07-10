#!/usr/bin/env bash
# Verify every libqrk_ffi.so under expo-cpu-scanner jniLibs has ELF LOAD
# segment alignment ≥ 2**14 (16 KB) — Google Play page-size requirement.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
JNI="$ROOT/expo-cpu-scanner/android/src/main/jniLibs"
MIN_POWER=14

if [[ ! -d "$JNI" ]]; then
  echo "error: no jniLibs at $JNI — run scripts/build-native-android.sh first" >&2
  exit 1
fi

# Prefer llvm-objdump from NDK, then host objdump.
find_objdump() {
  if [[ -n "${LLVM_OBJDUMP:-}" && -x "${LLVM_OBJDUMP}" ]]; then
    echo "$LLVM_OBJDUMP"
    return
  fi
  local roots=()
  [[ -n "${ANDROID_NDK_HOME:-}" ]] && roots+=("$ANDROID_NDK_HOME")
  [[ -n "${ANDROID_NDK_ROOT:-}" ]] && roots+=("$ANDROID_NDK_ROOT")
  local sdk="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Library/Android/sdk}}"
  if [[ -d "$sdk/ndk" ]]; then
    while IFS= read -r v; do roots+=("$sdk/ndk/$v"); done < <(ls -1 "$sdk/ndk" 2>/dev/null | sort -V)
  fi
  for root in "${roots[@]}"; do
    local pre="$root/toolchains/llvm/prebuilt"
    [[ -d "$pre" ]] || continue
    for host in "$pre"/*; do
      local cand="$host/bin/llvm-objdump"
      if [[ -x "$cand" ]]; then
        echo "$cand"
        return
      fi
    done
  done
  if command -v llvm-objdump >/dev/null 2>&1; then
    command -v llvm-objdump
    return
  fi
  if command -v objdump >/dev/null 2>&1; then
    command -v objdump
    return
  fi
  echo ""
}

OBJDUMP="$(find_objdump)"
if [[ -z "$OBJDUMP" ]]; then
  echo "error: need llvm-objdump or objdump (Android NDK or binutils)" >&2
  exit 1
fi

# Portable for macOS /bin/bash 3.2 (no mapfile).
SOS=()
while IFS= read -r line; do
  SOS+=("$line")
done <<EOF
$(find "$JNI" -type f -name 'libqrk_ffi.so' | sort)
EOF

if [[ ${#SOS[@]} -eq 0 ]]; then
  echo "error: no libqrk_ffi.so under $JNI" >&2
  exit 1
fi

fail=0
checked=0
for so in "${SOS[@]}"; do
  [[ -z "$so" ]] && continue
  # llvm-objdump -p prints program headers; only LOAD segments must be ≥16 KB.
  # Other headers (PHDR, DYNAMIC, RELRO, …) legitimately use smaller aligns.
  out="$("$OBJDUMP" -p "$so" 2>/dev/null || true)"
  powers=()
  while IFS= read -r line; do
    # Match e.g. "    LOAD off    0x0 ... align 2**14"
    if [[ "$line" =~ ^[[:space:]]*LOAD[[:space:]].*align[[:space:]]+2\*\*([0-9]+) ]]; then
      powers+=("${BASH_REMATCH[1]}")
    fi
  done <<EOF
$out
EOF

  if [[ ${#powers[@]} -eq 0 ]]; then
    echo "FAIL  $so — no LOAD alignments parsed (objdump: $OBJDUMP)" >&2
    fail=1
    continue
  fi

  bad=()
  for p in "${powers[@]}"; do
    if (( p < MIN_POWER )); then
      bad+=("2**$p")
    fi
  done
  rel="${so#"$ROOT/"}"
  if [[ ${#bad[@]} -gt 0 ]]; then
    echo "FAIL  $rel — LOAD align ${bad[*]} (need ≥ 2**$MIN_POWER)" >&2
    fail=1
  else
    joined="$(printf '2**%s ' "${powers[@]}")"
    echo "OK    $rel ($joined)"
    checked=$((checked + 1))
  fi
done

if (( fail )); then
  echo "Android 16 KB page-size check failed." >&2
  exit 1
fi
echo "Verified $checked libqrk_ffi.so file(s) for ≥16 KB ELF LOAD alignment."
