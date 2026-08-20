function cleecode_slice (name, r0, r1, c0, c1)
  ## CLEECODE_SLICE  Write a rectangle of one variable where CleeCode can read it.
  ##
  ## The workspace snapshot carries what a variable *is* — its shape, its range, a
  ## glimpse of it — because that is cheap to produce ten times a second for everything
  ## in the session. What it cannot carry is what a variable *contains*: a 2000x2000
  ## matrix is four million numbers, and nobody wants them written to disk on the chance
  ## that somebody looks.
  ##
  ## So the values are asked for, one screenful at a time. CleeCode types this at the
  ## prompt when you open the inspector or page around it, and reads the answer from the
  ## file. That is the round trip, and it is the same one the Octave GUI's variable
  ## editor makes — it simply does not have to write the numbers down in between.
  ##
  ## Bounds are clamped here rather than trusted: the caller last heard the size at the
  ## previous snapshot, and the variable may have been reassigned since.

  out = getenv ("CLEECODE_OCTAVE_SLICE");
  if (isempty (out))
    return;
  endif

  doc.name = name;
  doc.error = "";
  doc.rows = 0;
  doc.cols = 0;
  doc.r0 = 0;
  doc.c0 = 0;
  doc.data = {};
  doc.text = false;

  try
    v = evalin ("base", name);
    if (ischar (v))
      ## A char array is read as text, one row per line, rather than as a grid of
      ## character codes — which is what it looks like and not what it means.
      doc.text = true;
      doc.rows = rows (v);
      doc.cols = columns (v);
      lines = {};
      for k = 1:rows (v)
        lines{end+1} = v(k,:);
      endfor
      doc.data = lines;
    elseif (isnumeric (v) || islogical (v))
      [nr, nc] = size (v);
      doc.rows = nr;
      doc.cols = nc;
      r0 = max (1, min (r0, nr));
      c0 = max (1, min (c0, nc));
      r1 = max (r0, min (r1, nr));
      c1 = max (c0, min (c1, nc));
      doc.r0 = r0;
      doc.c0 = c0;
      block = double (v(r0:r1, c0:c1));
      grid = {};
      for k = 1:rows (block)
        ## A row at a time as a cell of numbers, for the same reason `vars` is a cell:
        ## jsonencode turns a single row into a bare list rather than a list of lists,
        ## and a 1xN variable would arrive shaped differently from every other.
        grid{end+1} = num2cell (block(k,:));
      endfor
      doc.data = grid;
    else
      doc.error = sprintf ("%s is a %s — no grid to show", name, class (v));
    endif
  catch err
    doc.error = err.message;
  end_try_catch

  tmp = sprintf ("%s.%d.tmp", out, getpid ());
  fid = fopen (tmp, "w");
  if (fid < 0)
    return;
  endif
  fputs (fid, jsonencode (doc));
  fclose (fid);
  rename (tmp, out);          # so a reader never sees half a file
endfunction
