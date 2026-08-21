#!/bin/sh
# Run this INSIDE a CleeCode terminal tab (the shell one), on the machine where it misbehaves.
echo "--- macchina ---"
uname -srm; echo "SSH: ${SSH_CONNECTION:+sì}${SSH_CONNECTION:-no}  DISPLAY: ${DISPLAY:-nessuno}"
echo "--- CleeCode ---"
clee --version 2>&1 | head -1
echo "WS   = ${CLEECODE_OCTAVE_WS:-NON IMPOSTATA}"
echo "LIB  = ${CLEECODE_OCTAVE_LIB:-NON IMPOSTATA}"
echo "OCTAVE_PATH = ${OCTAVE_PATH:-NON IMPOSTATA}"
echo "PYSTART     = ${PYTHONSTARTUP:-NON IMPOSTATA}"
[ -n "$CLEECODE_OCTAVE_LIB" ] && ls "$CLEECODE_OCTAVE_LIB" 2>&1 | tr '\n' ' ' && echo
[ -n "$CLEECODE_OCTAVE_WS" ] && ls -l "$CLEECODE_OCTAVE_WS" 2>&1 | tail -1
echo "--- interpreti ---"
octave --version 2>&1 | head -1 || echo "octave: assente"
python3 -V 2>&1
command -v gnuplot >/dev/null && echo "gnuplot: c'è" || echo "gnuplot: assente"
echo "--- cosa vede Octave ---"
octave --no-gui -q --eval '
  printf("hook trovato:  %d\n", exist("cleecode_ws_tick"));
  printf("boot eseguito: %s\n", get(0,"defaultfigurevisible"));
  printf("toolkit:       %s\n", strjoin(available_graphics_toolkits(), ","));
  printf("hook API:      %d\n", exist("add_input_event_hook"));
  printf("jsonencode:    %d\n", exist("jsonencode"));
' 2>&1 | grep -E "hook|boot|toolkit|json|error"
echo "--- cosa vede Python ---"
python3 -c '
import sys, os
print("ps1 sostituito:", type(sys.ps1).__name__ if hasattr(sys,"ps1") else "non interattivo")
print("audit hook:", hasattr(sys,"addaudithook"))
print("startup:", os.environ.get("PYTHONSTARTUP","NON IMPOSTATA"))
' 2>&1
