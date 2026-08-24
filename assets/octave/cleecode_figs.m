function figs = cleecode_figs (dir, dpi)
  ## CLEECODE_FIGS  Print the open figures to PNG, and describe where their axes are.
  ##
  ## Called from the workspace hook at a command boundary, so a figure is printed once
  ## per command rather than ten times a second.
  ##
  ## Only the figures that actually changed. Octave marks a figure "__modified__" when
  ## anything about it moves and lets that be reset, which is exactly the trigger
  ## needed: a session holding five plots while you work on one prints one. Without it
  ## every command pays for every figure — measured at 37 ms each for a line plot and
  ## 93 ms for a surface, which is the difference between a prompt that answers and one
  ## that hesitates.
  ##
  ## But "__modified__" is only Octave's answer under *gnuplot*. Under qt it is never
  ## set at all — measured 2026-08-22: after a fresh plot into an existing figure, after
  ## a title, after an xlim and after a zoom, it reads "off" every time. Left at that,
  ## a figure would be printed once and never again on any machine with a display, which
  ## is every desktop: zoom, pan and reset would send their command, the session would
  ## really zoom, and the tab would go on showing the first picture forever. That is what
  ## it did, and the check that should have caught it was passing without looking.
  ##
  ## So the two are asked together: the mark when it works, and otherwise a fingerprint
  ## of what would make the picture look different. The mark stays first because it is
  ## the cheaper question and it is right where it is answered.
  ##
  ## The size is forced through paperposition. A figure created with position
  ## [0 0 800 600] does *not* print to an 800x600 PNG: `print` sizes from the paper in
  ## inches and ignores the on-screen pixels, so it comes out at whatever
  ## paper x dpi happens to be. Every mouse coordinate mapped onto the result would be
  ## quietly wrong, which is worse than visibly wrong.

  persistent shape                      # the fingerprint each figure was last printed at

  figs = {};
  if (isempty (dir))
    return;
  endif
  if (isempty (shape))
    shape = struct ();
  endif
  if (nargin < 2 || isempty (dpi))
    dpi = 96;
  endif

  handles = get (0, "children");
  for k = numel (handles):-1:1          # oldest first, so figure 1 stays figure 1
    f = handles(k);
    try
      ## Back to invisible, whoever turned it on.
      ##
      ## `defaultfigurevisible` is off for this session, and that only decides how a figure is
      ## *born*: `figure(n)` on one that already exists raises it, which sets visible back to
      ## "on" for good. A script that draws into the same figure twice does that, and so did
      ## CleeCode itself until the nav commands stopped saying `figure(n)`. The result is the
      ## plot on screen twice — the tab, and a real window behind the terminal — which is
      ## precisely what the tab exists to avoid.
      ##
      ## Put right here rather than only at the places that raise it, because this is the one
      ## piece of code that sees every figure on every tick, and the ways a figure can come to
      ## be visible are not a list anybody can finish. Free when it is already off.
      if (strcmp (get (f, "visible"), "on"))
        set (f, "visible", "off");
      endif
      num = get (f, "number");
      png = fullfile (dir, sprintf ("fig%d.png", num));

      pos = get (f, "position");
      W = max (200, round (pos(3)));
      H = max (150, round (pos(4)));

      ## A figure that changed, one that looks different from the one on disk, or one
      ## whose PNG is not there — the last of which is how the first snapshot after
      ## CleeCode restarts still has pictures in it.
      key = sprintf ("f%d", num);
      now_shape = fingerprint (f);
      was_shape = "";
      if (isfield (shape, key))
        was_shape = shape.(key);
      endif
      if (strcmp (get (f, "__modified__"), "on") ...
          || ! strcmp (now_shape, was_shape) ...
          || ! exist (png, "file"))
        set (f, "paperunits", "inches", ...
                "paperposition", [0 0 W/dpi H/dpi], ...
                "papersize", [W/dpi H/dpi]);
        ## Drawn, printed, and taken away again — see cleecode_grid for why gnuplot needs
        ## the help and qt does not. Between these two lines the figure holds a few line
        ## objects the session never asked for, which is why nothing that reads the figure
        ## runs between them.
        undo = cleecode_grid (f);
        ## Printed beside the real name and moved onto it, because the editor is watching this
        ## file and reads it the moment it changes. A print straight onto the name is a picture
        ## that exists half-written for as long as the print takes, and a frame of an animation
        ## caught there decodes as "unexpected end of file" — the tab then says it could not
        ## read the picture, about a file that is perfectly good a millisecond later. A rename
        ## within a directory is atomic: the watcher sees the old picture or the new one.
        part = [png ".part.png"];
        cleecode_render (f, part, W, H, dpi);
        ## `rename` and not `movefile`: movefile shells out to `mv` for anything it does not
        ## recognise as trivial, and a fork per frame is a third of an animation's budget —
        ## measured here, it halved the frame rate. This is one call into the C library.
        [err, msg] = rename (part, png);
        if (err)
          error ("cleecode: could not put the figure in place: %s", msg);
        endif
        cleecode_grid_undo (undo);
        set (f, "__modified__", false);
        shape.(key) = now_shape;
      endif

      figs{end+1} = describe (f, num, png, W, H);
    catch
      ## A figure that will not print is not worth taking the session's panel down for.
    end_try_catch
  endfor
endfunction

function cleecode_render (f, part, W, H, dpi)
  ## Write the figure to `part` as a PNG W by H pixels, by the cheapest route that gets there.
  ##
  ## `print` is the route that works everywhere and it is expensive: measured on this machine at
  ## 155 ms a frame for a 500-point line at 800x600, and — the part that matters — 150 ms at
  ## 400x300 and 159 ms at half the resolution. The cost is not in drawing the pixels, it is
  ## print's own machinery: it copies the figure into a hidden one and takes the toolkit right
  ## round again for every frame. matplotlib's savefig does the same job in 11 ms, which is what
  ## an animation in Python against one in Octave looks like, and it was reported as exactly
  ## that.
  ##
  ## Under gnuplot there is a way round it. `drawnow (TERM, FILE)` hands the figure to the
  ## toolkit's own png terminal with none of that in between: measured 47 ms for the same frame,
  ## a little over three times faster, and it honours `linewidth`, which print through the eps
  ## path does not. The grid is still ours (see cleecode_grid, called before this): those are
  ## real line objects in the figure, so they are drawn whichever route the picture takes, and
  ## the picture is the same one it was — only sooner.
  ##
  ## Only under gnuplot, and only when what lands on disk really is the PNG that was asked for.
  ## Under qt the terminal argument is ignored and the file written is PostScript with a .png
  ## name — checked here on the first four bytes, because a tab handed PostScript says it cannot
  ## read the picture and there is nothing on screen to say why. The size is checked too: `print`
  ## is told the size through paperposition and this is not, so a figure whose position is
  ## smaller than the floor the caller applies would come out at the wrong number of pixels, and
  ## every mouse coordinate mapped onto it would be quietly wrong. Either check failing is not an
  ## error — it is a print, exactly as before.
  if (strcmp (get (f, "__graphics_toolkit__"), "gnuplot"))
    try
      drawnow ("png", part);
      [ok, w, h] = cleecode_png_size (part);
      if (ok && w == W && h == H)
        return;
      endif
    catch
      ## An Octave that does not take a terminal here, or a figure it will not draw that way.
    end_try_catch
  endif
  print (f, "-dpng", sprintf ("-r%d", dpi), part);
endfunction

function [ok, w, h] = cleecode_png_size (path)
  ## The magic number and the size out of a PNG's header, without decoding it.
  ##
  ## A PNG opens with eight fixed bytes and then IHDR, whose first two fields are the width and
  ## the height as big-endian 32-bit integers — bytes 17 to 24 of the file. Reading them costs
  ## one open and 24 bytes, against decoding a picture to ask how big it is.
  ok = false; w = 0; h = 0;
  fid = fopen (path, "rb");
  if (fid < 0)
    return;
  endif
  head = fread (fid, 24, "uint8")';
  fclose (fid);
  if (numel (head) < 24 || ! isequal (head(1:4), [137 80 78 71]))
    return;
  endif
  w = head(17)*2^24 + head(18)*2^16 + head(19)*2^8 + head(20);
  h = head(21)*2^24 + head(22)*2^16 + head(23)*2^8 + head(24);
  ok = true;
endfunction

function s = fingerprint (f)
  ## What would make a figure look different, in the cheapest terms that catch it.
  ##
  ## The figure's own size, and for each axes: the limits, the view, where it sits, how
  ## many things are drawn in it, its title — and a sample of what those things hold.
  ## Between them these cover what the navigation keys do (limits and view), what a replot
  ## does (the child count and usually the limits), what labelling does, and what changing
  ## the data in place does. It is a handful of `get` calls per figure per command — next
  ## to nothing beside the 37 ms a print costs, which is the whole reason for asking.
  ##
  ## The sample is the late addition, and it is the difference between a picture that
  ## follows the session and one that stops at its first frame. Everything above is
  ## *geometry*: an animation — `set(h, "ydata", ...)` in a loop, with the axes pinned by
  ## `axis(...)` so nothing moves — changes none of it. The fingerprint came out identical
  ## every time round, so the figure was never reprinted, and the tab showed frame one
  ## after the loop had finished and for the rest of the session. Nothing said so: the
  ## picture was a real picture of a real figure, just not of that one any more.
  ##
  ## Not a hash of the PNG: that would mean printing it to find out whether to print it.
  ## And not the whole of the data either — a surface is a million numbers and this runs
  ## ten times a second. Thirty-two points spread through it costs the same whether the
  ## array holds a hundred or a million, and there is no realistic redraw that moves none
  ## of them while leaving every count and limit alone.

  parts = {sprintf("%g,", get (f, "position"))};
  ax = get (f, "children");
  for k = 1:numel (ax)
    a = ax(k);
    try
      if (! strcmp (get (a, "type"), "axes"))
        continue;
      endif
      t = "";
      try
        t = get (get (a, "title"), "string");
        if (! ischar (t))
          t = "";
        endif
      catch
      end_try_catch
      kids = get (a, "children");
      parts{end+1} = sprintf ("%g,", get (a, "xlim"), get (a, "ylim"), get (a, "zlim"), ...
                              get (a, "view"), get (a, "position"), numel (kids));
      parts{end+1} = t;
      for j = 1:numel (kids)
        parts{end+1} = drawn_data (kids(j));
      endfor
    catch
      ## An axes that will not answer is not worth taking the snapshot down for.
    end_try_catch
  endfor
  s = strjoin (parts, "|");
endfunction

function d = describe (f, num, png, W, H)
  ## The geometry a pane pixel needs to become a data coordinate, so navigation can be
  ## worked out without a round trip to the interpreter per mouse move.
  d.fig = num;
  d.path = png;
  d.png = [W H];
  axes_list = {};
  kids = get (f, "children");
  for k = numel (kids):-1:1
    ax = kids(k);
    if (! strcmp (get (ax, "type"), "axes"))
      continue;
    endif
    a.pos = get (ax, "position");     # normalised to the figure, origin bottom-left
    a.xlim = get (ax, "xlim");
    a.ylim = get (ax, "ylim");
    a.xscale = get (ax, "xscale");    # a log axis maps through log10, not linearly
    a.yscale = get (ax, "yscale");
    v = get (ax, "view");
    ## Octave has no "is this 3-D" flag. A plain 2-D axes looks straight down at [0 90];
    ## anything else is a view somebody set.
    a.is3d = ! (numel (v) == 2 && abs (v(1)) < 1e-9 && abs (v(2) - 90) < 1e-9);
    a.view = v;
    axes_list{end+1} = a;
  endfor
  ## A cell array, for the same reason `vars` is one: jsonencode turns a 1x1 struct
  ## array into a bare object rather than a one-element array, so a figure with exactly
  ## one axes would serialise to a different shape than every other.
  d.axes = axes_list;
endfunction

function s = drawn_data (h)
  ## A cheap sample of what one drawn thing holds, for [`fingerprint`].
  ##
  ## Thirty-two points at most out of each array, so a surface of a million numbers costs the
  ## same as a line of a hundred. The count goes in too, because a plot that grew a point is a
  ## plot that changed even if every sampled value happens to land the same.
  ##
  ## Everything is inside its own try: a child may be a legend, a text object or something with
  ## no data at all, and the tick must not fall over on one to leave every other figure unprinted.

  s = "";
  for prop = {"xdata", "ydata", "zdata", "cdata", "string"}
    try
      v = get (h, prop{1});
      if (ischar (v))
        s = [s, v, ","];
        continue;
      endif
      v = v(:);
      n = numel (v);
      if (n == 0)
        continue;
      endif
      at = unique (round (linspace (1, n, min (n, 32))));
      s = [s, sprintf("%d:", n), sprintf("%g,", v(at))];
    catch
      ## Not a thing with that property. The next one may be.
    end_try_catch
  endfor
endfunction
