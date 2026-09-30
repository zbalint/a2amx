#!/bin/sh
# Test double for the Claude Code composer. Env: OUT (output file prefix),
# PASTE_LEN (byte length of the paste to expect), MODE (ready | dialog_after_paste).
size=$(stty size)
cols=${size#* }
rule=
i=0
while [ "$i" -lt "$cols" ]; do rule="$rule─"; i=$((i + 1)); done
printf '\033[2J\033[?2004h\033[2;1H%s\033[3;1H❯ \033[4;1H%s\033[3;3H\033[?25h' "$rule" "$rule"
stty raw -echo
dd bs=1 count="$PASTE_LEN" of="$OUT.paste.tmp" 2>/dev/null
mv "$OUT.paste.tmp" "$OUT.paste"
t1=$(date +%s%N)
if [ "$MODE" = dialog_after_paste ]; then
  printf '\033[2J\033[H Enter to confirm · Esc to cancel'
fi
stty min 0 time 20
dd bs=1 count=1 of="$OUT.cr" 2>/dev/null
t2=$(date +%s%N)
echo $(((t2 - t1) / 1000000)) > "$OUT.gap_ms"
sleep 30
