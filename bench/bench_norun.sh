#!/usr/bin/env bash
# 「模拟器未启动」失败路径的自动识别耗时。
#
# 这是自动识别链路里最容易被忽视的一段：模拟器没开时，两版都要先确认"确实没有
# 可用的模拟器"再报错。如果这段时间很长，用户会以为程序卡住。
#
# 前置：先手动关闭模拟器（不要在本脚本里关，避免误关用户正在用的实例）
#   "F:\Program Files\Netease\MuMu\nx_main\MuMuManager.exe" control -v 0 shutdown
#
# 用法：bash bench/bench_norun.sh
# 产出：bench/results/norun.csv
#
# 想对比"改前 / 改后"，可以把 RS_EXE 指向旧版本的 Rust 二进制：
#   RS_EXE=/path/to/old/MuMuShot.exe bash bench/bench_norun.sh
set -u

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PY_EXE="$ROOT/MuMuShot.exe"
RS_EXE="${RS_EXE:-$ROOT/rust/target/release/MuMuShot.exe}"
RUNS="${RUNS:-5}"
WARMUP=1
OUT="$ROOT/bench/results"
mkdir -p "$OUT"
RAW="$OUT/norun.csv"

for f in "$PY_EXE" "$RS_EXE"; do
  [ -f "$f" ] || { echo "找不到可执行文件：$f"; exit 1; }
done

# 前置检查：端口必须已经释放，否则测到的是成功路径
if netstat -ano 2>/dev/null | grep -qE "127\.0\.0\.1:(16384|7555|5555)\b.*LISTENING|LISTENING.*127\.0\.0\.1:(16384|7555|5555)\b"; then
  echo "检测到 adb 端口仍在监听 —— 请先关闭模拟器再跑本脚本："
  netstat -ano 2>/dev/null | grep -E "127\.0\.0\.1:(16384|7555|5555)\b"
  exit 1
fi

now_ms() { date +%s%N; }
median() { sort -n | awk '{a[NR]=$1} END{if(NR==0){print "NA";exit} if(NR%2){print a[(NR+1)/2]} else {print (a[NR/2]+a[NR/2+1])/2}}'; }

# 计时并取退出码，输出 "毫秒,退出码"
time_code() {
  local s e c
  s=$(now_ms)
  "$@" >/dev/null 2>&1
  c=$?
  e=$(now_ms)
  echo "$(( (e - s) / 1000000 )),$c"
}

echo "impl,run,ms,exit" > "$RAW"

for impl in python rust; do
  [ "$impl" = python ] && exe="$PY_EXE" || exe="$RS_EXE"
  for _ in $(seq 1 $WARMUP); do $exe --full >/dev/null 2>&1; done
done

for i in $(seq 1 "$RUNS"); do
  if [ $((i % 2)) -eq 1 ]; then order="python rust"; else order="rust python"; fi
  for impl in $order; do
    [ "$impl" = python ] && exe="$PY_EXE" || exe="$RS_EXE"
    res=$(time_code "$exe" --full)
    echo "$impl,$i,${res}" >> "$RAW"
  done
done

echo "== 模拟器未启动时的自动识别耗时（毫秒，n=$RUNS） =="
printf "%-10s %-10s %-10s %s\n" "实现" "中位数" "最快" "退出码"
for impl in python rust; do
  ms=$(awk -F, -v i="$impl" '$1==i {print $3}' "$RAW" | median)
  mn=$(awk -F, -v i="$impl" '$1==i {print $3}' "$RAW" | sort -n | head -1)
  ec=$(awk -F, -v i="$impl" '$1==i {print $4}' "$RAW" | sort -u | tr '\n' '/')
  printf "%-10s %-10s %-10s %s\n" "$impl" "$ms" "$mn" "$ec"
done
echo
echo "预期退出码 3（连不上设备）；两版都应以「模拟器没在运行」类文案快速失败。"
