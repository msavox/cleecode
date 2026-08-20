function cleecode_boot ()
  ## CLEECODE_BOOT  Start publishing this session's workspace, if CleeCode asked for it.
  ##
  ## Run by the `clee -w octave` preset as the interpreter starts. It exists so the
  ## startup command stays short and readable where the user can see it — the whole
  ## of the alternative is one long --eval that installs the hook inline, and that
  ## line is echoed by the shell into their transcript and stays there.
  ##
  ## Outside CleeCode the variable is unset and this does nothing, so a copy of the
  ## file on somebody's path is inert rather than surprising.

  ws = getenv ("CLEECODE_OCTAVE_WS");
  if (isempty (ws))
    return;
  endif

  ## No figure window ever opens. Reparenting a live Qt window into a terminal is not
  ## possible, so a plot reaches CleeCode as a picture instead — and a window appearing
  ## behind the terminal, which is what happens without this, is the worst of both.
  set (0, "defaultfigurevisible", "off");
  cleecode_ws (ws);
endfunction
