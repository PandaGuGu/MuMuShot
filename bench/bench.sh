#!/usr/bin/env bash
# MuMuShot 性能评测：Python 版 vs Rust 版，同一口径、同一台机器。
#
# 用法：
#   bash bench/bench.sh              # 默认 7 次取中位数
#   RUNS=15 bash bench/bench.sh      # 自定义次数
#
# 产出：bench/results/raw.csv（每次原始数据）
set -u

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PY_EXE="$ROOT/MuMuShot.exe"
RS_EXE="${RS_EXE:-$ROOT/rust/target/release/MuMuShot.exe}"
RUNS="${RUNS:-7}"
WARMUP=2
OUT="$ROOT/bench/results"
mkdir -p "$OUT"
RAW="$OUT/raw.csv"

if [ ! -f "$PY_EXE" ]; then echo "找不到 Python 版：$PY_EXE"; exit 1; fi
if [ ! -f "$RS_EXE" ]; then echo "找不到 Rust 版：$RS_EXE（先 cargo build --release）"; exit 1; fi

now_ms() { date +%s%N; }

# 单次计时（毫秒）
time_once() {
  local s e
  s=$(now_ms)
  "$@" >/dev/null 2>&1
  e=$(now_ms)
  echo $(( (e - s) / 1000000 ))
}

median() {
  sort -n | awk '{a[NR]=$1} END{if(NR==0){print "NA";exit} if(NR%2){print a[(NR+1)/2]} else {print (a[NR/2]+a[NR/2+1])/2}}'
}

echo "impl,case,run,ms" > "$RAW"

run_case() {
  local impl="$1" exe="$2" case="$3"; shift 3
  # 预热
  for _ in $(seq 1 $WARMUP); do time_once "$exe" "$@" >/dev/null; done
  for i in $(seq 1 $RUNS); do
    local ms
    ms=$(time_once "$exe" "$@")
    echo "$impl,$case,$i,$ms" >> "$RAW"
  done
}

echo "== 采集 Python 版 =="
run_case python "$PY_EXE" selftest  --selftest
run_case python "$PY_EXE" listwin   --list-windows
run_case python "$PY_EXE" shot1x1   --region 0,0,1,1 --device 127.0.0.1:16384
run_case python "$PY_EXE" shotfull  --full --device 127.0.0.1:16384

echo "== 采集 Rust 版 =="
run_case rust "$RS_EXE" selftest  --selftest
run_case rust "$RS_EXE" listwin   --list-windows
run_case rust "$RS_EXE" shot1x1   --region 0,0,1,1 --device 127.0.0.1:16384
run_case rust "$RS_EXE" shotfull  --full --device 127.0.0.1:16384

echo
echo "== 中位数（毫秒） =="
printf "%-10s %-10s %s\n" "用例" "Python" "Rust"
for case in selftest listwin shot1x1 shotfull; do
  p=$(awk -F, -v c="$case" '$1=="python" && $2==c {print $4}' "$RAW" | median)
  r=$(awk -F, -v c="$case" '$1=="rust" && $2==c {print $4}' "$RAW" | median)
  printf "%-10s %-10s %s\n" "$case" "$p" "$r"
done

echo
echo "== 发行体积 =="
py_files=$(find "$ROOT/MuMuShot.exe" "$ROOT/_internal" -type f 2>/dev/null | wc -l)
py_bytes=$(du -sb "$ROOT/MuMuShot.exe" "$ROOT/_internal" 2>/dev/null | awk '{s+=$1} END{print s}')
rs_dir=$(dirname "$RS_EXE")
rs_files=$(find "$rs_dir" -maxdepth 1 -type f -name "*.exe" -o -maxdepth 1 -type f -name "*.dll" 2>/dev/null | wc -l)
rs_bytes=$(stat -c "%s" "$RS_EXE")
printf "Python 版：%s 字节 / %s 个文件\n" "$py_bytes" "$py_files"
printf "Rust   版：%s 字节 / %s 个文件\n" "$rs_bytes" "$rs_files"
awk -v p="$py_bytes" -v r="$rs_bytes" 'BEGIN{printf "体积比：Python 是 Rust 的 %.1f 倍\n", p/r}'
