#!/usr/bin/env bash
# Stands in for `clee` on PATH while docs/demo.tape records. The point is that the only command
# on camera is `clee`, with no argument, exactly as anyone would start it — the tmux session the
# recording actually needs is set up in here, out of shot.
#
# The real binary sits beside this script as `clee-real`. Environment set by the tape:
#   CLEE_DEMO_REPO  the checkout, so the key script can be found
#   CLEE_DEMO_CONF  a throwaway config dir, also holding the tmux.conf that hides tmux's status
#   CLEE_DEMO_KEYS  the key script to play into the session
set -u

# The keys start a moment after the session does, so the splash screen is on camera first.
( sleep 1.5; bash "$CLEE_DEMO_KEYS" demo ) &

exec tmux -f "$CLEE_DEMO_CONF/tmux.conf" \
    new-session -s demo -x 172 -y 48 \
    -e "XDG_CONFIG_HOME=$CLEE_DEMO_CONF" \
    clee-real
