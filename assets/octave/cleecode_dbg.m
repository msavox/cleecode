function state = cleecode_dbg ()
  ## CLEECODE_DBG  Apply the breakpoints CleeCode asked for, and report where we are stopped.
  ##
  ## Both halves run from the idle hook, which means neither types anything at the
  ## user's prompt. Measured, and it is why this is shaped the way it is:
  ##
  ##   · `dbstop` works through evalin from inside the hook, so a breakpoint set in the
  ##     editor never appears in the transcript;
  ##   · the hook keeps firing at the `debug>` prompt, so being stopped is something
  ##     CleeCode can see rather than something the user has to report;
  ##   · `dbstack` from in here carries the hook's own frame on top, so it is dropped;
  ##   · `dbstep` from in here returns without an error and does not move. Stepping is
  ##     the one thing that has to be typed at the debug prompt, and the manual says so
  ##     rather than offering a key that quietly does nothing.

  persistent seen
  state.stopped = false;
  state.name = "";
  state.file = "";
  state.line = 0;
  state.stack = {};

  ## --- the breakpoints CleeCode wants -------------------------------------------------
  log = getenv ("CLEECODE_DBG_LOG");
  req = getenv ("CLEECODE_OCTAVE_BREAK");
  if (! isempty (req) && exist (req, "file") == 2)
    info = dir (req);
    if (! isempty (info) && (isempty (seen) || seen != info(1).datenum))
      seen = info(1).datenum;
      try
        ask = jsondecode (fileread (req));
        evalin ("base", "dbclear all");
        for k = 1:numel (ask)
          one = ask(k);
          if (iscell (one)); one = one{1}; endif
          ## By function name, which is what dbstop takes: for a script or a function
          ## file they are the same word, and it is the one Octave knows it by.
          evalin ("base", sprintf ("dbstop in %s at %d", one.name, one.line));
        endfor
      catch err
        ## A half-written request, or a breakpoint in a file Octave cannot find. Neither
        ## is worth taking the session's panel down for.
        dbg = getenv ("CLEECODE_DBG_LOG");
        if (! isempty (dbg))
          fid = fopen (dbg, "a"); fprintf (fid, "breakpoint: %s\n", err.message); fclose (fid);
        endif
      end_try_catch
    endif
  endif

  ## --- where we are, if we are stopped ------------------------------------------------
  if (! isdebugmode ())
    return;
  endif
  st = dbstack ();
  ## Frame 1 is this function, frame 2 is the hook that called it: both are CleeCode's
  ## own scaffolding and neither is anywhere the user's code is.
  frames = {};
  for k = 1:numel (st)
    if (strncmp (st(k).name, "cleecode_", 9))
      continue;
    endif
    frames{end+1} = struct ("name", st(k).name, "file", st(k).file, "line", st(k).line);
  endfor
  if (isempty (frames))
    return;
  endif
  state.stopped = true;
  state.name = frames{1}.name;
  state.file = frames{1}.file;
  state.line = frames{1}.line;
  state.stack = frames;
endfunction
