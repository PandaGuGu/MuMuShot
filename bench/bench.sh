#!/usr/bin/env bash
# MuMuShot 性能评测：Python 版 vs Rust 版，同一口径、同一台机器。
#
# 用法：
#   bash bench/bench.sh              # 默认 7 次取中位数
#   RUNS=9 bash bench/bench.sh       # 自定义次数
#
# 产出：bench/results/raw.csv（每次原始数据）
#
# 采集方式：两种实现**交替执行**（第 N 次 python 先、Rust 后；第 N+1 次反过来）。
# 端到端用例有约 ±10% 波动，主要是 adb 与模拟器响应在漂移；若先把一种实现跑完
# 再跑另一种，漂移会整体算到某一方头上（实测会出现"截图更小的用例反而更慢"的
# 假象）。交替 + 双向排序可以让漂移平摊到两边。
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

# 用例：名字 -> 参数。discover 专门隔离"自动识别模拟器端口"的开销
CASES=(selftest listwin shot1x1 shotfull discover)
case_args() {
  case "$1" in
    selftest) echo "--selftest" ;;
    listwin)  echo "--list-windows" ;;
    shot1x1)  echo "--region 0,0,1,1 --device 127.0.0.1:16384" ;;
    shotfull) echo "--full --device 127.0.0.1:16384" ;;
    discover) echo "--full" ;;
    *) echo "" ;;
  esac
}
exe_of() { if [ "$1" = python ]; then echo "$PY_EXE"; else echo "$RS_EXE"; fi; }

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

echo "== 预热（各 $WARMUP 轮） =="
for _ in $(seq 1 $WARMUP); do
  for case in "${CASES[@]}"; do
    for impl in python rust; do
      time_once "$(exe_of "$impl")" $(case_args "$case") >/dev/null
    done
  done
done

echo "== 采集（交替执行，共 $RUNS 轮） =="
for i in $(seq 1 "$RUNS"); do
  for case in "${CASES[@]}"; do
    # 奇偶轮交换先后顺序，抵消"谁先跑谁吃亏"
    if [ $((i % 2)) -eq 1 ]; then order="python rust"; else order="rust python"; fi
    for impl in $order; do
      ms=$(time_once "$(exe_of "$impl")" $(case_args "$case"))
      echo "$impl,$case,$i,$ms" >> "$RAW"
    done
  done
  printf "\r  第 %s/%s 轮" "$i" "$RUNS"
done
echo

echo
echo "== 中位数（毫秒） =="
printf "%-10s %-10s %-10s %s\n" "用例" "Python" "Rust" "Rust 快"
for case in "${CASES[@]}"; do
  p=$(awk -F, -v c="$case" '$1=="python" && $2==c {print $4}' "$RAW" | median)
  r=$(awk -F, -v c="$case" '$1=="rust" && $2==c {print $4}' "$RAW" | median)
  ratio=$(awk -v p="$p" -v r="$r" 'BEGIN{if(r>0) printf "%.2f×", p/r; else print "-"}')
  printf "%-10s %-10s %-10s %s\n" "$case" "$p" "$r" "$ratio"
done

echo
echo "== 派生量（毫秒，中位数之差） =="
p1=$(awk -F, '$1=="python" && $2=="shot1x1" {print $4}' "$RAW" | median)
pf=$(awk -F, '$1=="python" && $2=="shotfull" {print $4}' "$RAW" | median)
r1=$(awk -F, '$1=="rust" && $2=="shot1x1" {print $4}' "$RAW" | median)
rf=$(awk -F, '$1=="rust" && $2=="shotfull" {print $4}' "$RAW" | median)
pd=$(awk -F, '$1=="python" && $2=="discover" {print $4}' "$RAW" | median)
rd=$(awk -F, '$1=="rust" && $2=="discover" {print $4}' "$RAW" | median)
awk -v a="$pf" -v b="$p1" 'BEGIN{printf "全屏截图+编码增量：Python %d\n", a-b}'
awk -v a="$rf" -v b="$r1" 'BEGIN{printf "全屏截图+编码增量：Rust   %d\n", a-b}'
awk -v a="$pd" -v b="$pf" 'BEGIN{printf "自动识别设备开销：Python %d\n", a-b}'
awk -v a="$rd" -v b="$rf" 'BEGIN{printf "自动识别设备开销：Rust   %d\n", a-b}'

echo
echo "== 发行体积 =="
py_files=$(find "$ROOT/MuMuShot.exe" "$ROOT/_internal" -type f 2>/dev/null | wc -l)
py_bytes=$(du -sb "$ROOT/MuMuShot.exe" "$ROOT/_internal" 2>/dev/null | awk '{s+=$1} END{print s}')
rs_bytes=$(stat -c "%s" "$RS_EXE")
printf "Python 版：%s 字节 / %s 个文件\n" "$py_bytes" "$py_files"
printf "Rust   版：%s 字节 / 1 个文件\n" "$rs_bytes"
awk -v p="$py_bytes" -v r="$rs_bytes" 'BEGIN{printf "体积比：Python 是 Rust 的 %.1f 倍\n", p/r}'
