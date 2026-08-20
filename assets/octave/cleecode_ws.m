function cleecode_ws (arg)
  ## CLEECODE_WS  Publish the base workspace as JSON, for CleeCode's panel.
  ##
  ##   cleecode_ws ("/path/to/snapshot.json")  start watching, write there
  ##   cleecode_ws ("off")                     stop watching
  ##
  ## Installs an input event hook, which Octave calls while it waits at the
  ## prompt — the same idle moment the Octave GUI refreshes its own workspace
  ## dock from. The hook prints nothing: it writes a JSON snapshot to a file,
  ## atomically, and CleeCode watches that file's mtime. Nothing appears in the
  ## user's transcript, and a busy interpreter is simply not interrupted,
  ## because the hook does not run at all while a command is executing.

  persistent hook_id

  if (nargin < 1 || strcmp (arg, "off"))
    if (! isempty (hook_id))
      remove_input_event_hook (hook_id);
      hook_id = [];
    endif
    return;
  endif

  if (! isempty (hook_id))
    remove_input_event_hook (hook_id);
  endif
  cleecode_ws_tick ("reset");
  hook_id = add_input_event_hook (@cleecode_ws_tick, arg);
endfunction
