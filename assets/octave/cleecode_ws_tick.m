function cleecode_ws_tick (out)
  ## The input event hook itself. Called roughly every 100 ms while Octave is
  ## waiting for input, so it has to decide cheaply whether anything happened
  ## before doing any real work.
  ##
  ## An error in here would make Octave drop the hook, so everything is inside
  ## a try/catch that swallows: a broken panel must not break the session.

  persistent seq fp last_call

  if (ischar (out) && strcmp (out, "reset"))
    seq = 0; fp = ""; last_call = 0;
    return;
  endif

  try
    if (isempty (seq)) seq = 0; fp = ""; last_call = 0; endif

    ## Answered before anything else, and whether or not the workspace moved: it is a
    ## question CleeCode asked, and the answer is wanted now rather than at the next
    ## command. Nothing is typed at the user's prompt to ask it — that was the first rule
    ## this design set itself, and reading a request file keeps it.
    cleecode_answer_slice ();
    dbg = cleecode_dbg ();

    now = time ();
    gap = now - last_call;
    last_call = now;

    ## Three independent reasons to believe the workspace may have moved:
    ##
    ##  · the history grew, or its last line changed — one entry per command
    ##    entered, which catches commands too fast to leave a gap and in-place
    ##    edits (a(1)=99) that leave every metadatum identical. Comparing the
    ##    last line as well as the count matters once history_size saturates
    ##    and the count stops growing;
    ##  · the interpreter was away long enough to have run something;
    ##  · the whos metadata differ — covers changes made by anything other than
    ##    a typed command, a script's last line included.
    h = evalin ("base", "history ()");
    ## Stopped in the debugger, the variables that matter are the *frame's*, not the base
    ## workspace's — that is the whole difference between watching a program run and
    ## looking at what is left after it. Measured: `evalin("caller", ...)` from in here
    ## reaches the stopped frame, while `evalin("base", ...)` does not.
    scope = "base";
    if (dbg.stopped)
      scope = "caller";
    endif
    w = evalin (scope, "whos");
    ## The values come from the *same* scope as the names, which sounds obvious and was
    ## not: reading names from the stopped frame and values from the base workspace makes
    ## every read fail with "undefined", and the tick's own catch then swallows the whole
    ## snapshot. The only symptom was a panel that stopped updating.
    vals = cell (1, numel (w));
    for k = 1:numel (w)
      try
        vals{k} = evalin (scope, w(k).name);
      catch
        vals{k} = [];
      end_try_catch
    endfor
    last_cmd = ""; if (! isempty (h)) last_cmd = h{end}; endif
    ## One vectorised sprintf per field, letting the cs-list of the struct
    ## array expand into it. Growing a string in a loop instead costs O(n^2)
    ## and reached 7 ms per tick at 200 variables — 7% of a core, spent
    ## forever, just sitting at the prompt.
    now_fp = sprintf ("%d\n%s\n%d:%s:%d\n%s\n%s\n%s\n%s", numel (h), last_cmd, ...
                      dbg.stopped, dbg.name, dbg.line, ...
                      sprintf ("%s,", w.name), sprintf ("%d,", w.bytes), ...
                      sprintf ("%s,", w.class), sprintf ("%dx", w.size));

    if (strcmp (now_fp, fp) && gap < 0.30 && seq > 0)
      return;
    endif
    fp = now_fp;
    seq++;

    ## Figures are printed here rather than on every tick, because here *is* the command
    ## boundary: the fingerprint above only differs when something ran.
    figs = cleecode_figs (getenv ("CLEECODE_OCTAVE_FIGS"));
    write_snapshot (out, seq, now, w, vals, figs, recent (h), dbg);
  catch err
    ## Silent by default: a panel that fails to draw must never take the session with it.
    ##
    ## But silent has a price, and it was paid twice while this was being written — once
    ## for a function that was not shipped with the binary, once for names read from one
    ## scope and values from another. Both times the only symptom was a panel that never
    ## filled in, with nothing anywhere saying why. Set CLEECODE_DBG_LOG to a path and the
    ## reason lands there.
    dbglog = getenv ("CLEECODE_DBG_LOG");
    if (! isempty (dbglog))
      fid = fopen (dbglog, "a");
      fprintf (fid, "tick fallito: %s\n", err.message);
      fclose (fid);
    endif
  end_try_catch
endfunction

function cleecode_answer_slice ()
  ## Reads the request CleeCode leaves beside the snapshot and answers it, at most once
  ## per change. Both files are written by rename, so neither side ever sees half of one.
  persistent seen

  req = getenv ("CLEECODE_OCTAVE_SLICE_REQ");
  if (isempty (req) || exist (req, "file") != 2)
    return;
  endif
  info = dir (req);
  if (isempty (info))
    return;
  endif
  stamp = info(1).datenum;
  if (! isempty (seen) && seen == stamp)
    return;
  endif
  seen = stamp;
  try
    ask = jsondecode (fileread (req));
    cleecode_slice (ask.name, ask.r0, ask.r1, ask.c0, ask.c1);
  catch
    ## A half-written or unreadable request is not worth taking the panel down for.
  end_try_catch
endfunction

function out = recent (h)
  ## The last few things the user typed, newest last, with CleeCode's own injections left
  ## out. Everything this program types at the prompt ends in a marker comment for exactly
  ## this: a list of recent commands full of `figure(1); zoom(2);` is a list of what
  ## CleeCode did, which nobody asked to see. Matched on the comment rather than on the
  ## shape of the command, so somebody typing `figure(2)` themselves is never mistaken
  ## for us.
  out = {};
  if (isempty (h))
    return;
  endif
  for k = numel (h):-1:1
    line = strtrim (h{k});
    if (isempty (line) || ! isempty (strfind (line, "%cleecode")))
      continue;
    endif
    out{end+1} = line;
    if (numel (out) >= 12)
      break;
    endif
  endfor
  out = fliplr (out);          # newest last, the way a transcript reads
endfunction

function write_snapshot (out, seq, now, w, vals, figs, history, dbg)
  ## Elements above which min/max/mean are skipped rather than paid for ten
  ## times a second. A 2000x2000 matrix is 4e6 elements; scanning it at every
  ## prompt would be visible.
  STAT_LIMIT = 1e6;
  PREVIEW_ELEMS = 8;

  ## A cell array, not a struct array: jsonencode turns a 1x1 struct array into
  ## a bare object rather than a one-element array, so a workspace holding a
  ## single variable would serialise to a different shape than any other. A
  ## cell array is always an array, empty included.
  vars = {};

  for k = 1:numel (w)
    v = struct ("name", w(k).name, "class", w(k).class, "size", w(k).size, ...
                "bytes", w(k).bytes, "attr", attrs (w(k)), ...
                "min", NaN, "max", NaN, "mean", NaN, "nans", 0, "preview", "");

    val = vals{k};

    if (isnumeric (val) || islogical (val))
      if (isempty (val))
        v.preview = "[]";
      elseif (numel (val) > STAT_LIMIT)
        v.preview = "(too large to summarise)";
      elseif (iscomplex (val))
        a = abs (val(:));
        v.min = min (a); v.max = max (a); v.mean = mean (a);
        v.preview = "|z|";
      else
        d = double (val(:));
        v.nans = sum (isnan (d));
        d = d(! isnan (d));
        if (! isempty (d))
          v.min = min (d); v.max = max (d); v.mean = mean (d);
        endif
        v.preview = clip (mat2str (val(1:min (PREVIEW_ELEMS, numel (val)))'), 40);
      endif
    elseif (ischar (val))
      v.preview = clip (val(1,:), 40);
    elseif (iscell (val))
      v.preview = sprintf ("%d elements", numel (val));
    elseif (isstruct (val))
      f = fieldnames (val);
      v.preview = clip (strjoin (f', ", "), 40);
    elseif (is_function_handle (val))
      v.preview = clip (func2str (val), 40);
    endif

    vars{end+1} = v;
  endfor

  ## Built field by field: struct() would read the cell array as a request for
  ## a struct array of that shape.
  doc.v = 1;
  ## Which interpreter wrote this. The reader is one piece of code for both languages and the
  ## field is how it knows whose workspace it is looking at; without it the view can only call
  ## it "workspace". (The shared handoff claimed both sides already emitted this. Octave did
  ## not.)
  doc.lang = "octave";
  doc.seq = seq;
  doc.time = now;
  doc.pid = getpid ();
  doc.cwd = pwd ();
  doc.vars = vars;
  doc.figures = figs;
  doc.history = history;
  doc.debug = dbg;

  ## Write beside the target and rename, so a reader never sees half a file.
  tmp = sprintf ("%s.%d.tmp", out, getpid ());
  fid = fopen (tmp, "w");
  if (fid < 0) return; endif
  fputs (fid, jsonencode (doc));
  fclose (fid);
  rename (tmp, out);
endfunction

function s = attrs (v)
  ## Same letters whos prints in its Attr column.
  s = "";
  if (v.complex)    s = [s "c"]; endif
  if (v.sparse)     s = [s "s"]; endif
  if (v.global)     s = [s "g"]; endif
  if (v.persistent) s = [s "p"]; endif
endfunction

function s = clip (s, n)
  s = strtrim (s);
  if (numel (s) > n)
    s = [s(1:n-3) "..."];
  endif
endfunction
