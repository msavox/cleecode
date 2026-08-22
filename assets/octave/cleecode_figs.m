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
        ## Drawn, printed, and taken away again — see minor_grid below for why gnuplot
        ## needs the help and qt does not. Between these two lines the figure holds a few
        ## line objects the session never asked for, which is why nothing that reads the
        ## figure runs between them.
        undo = minor_grid (f);
        print (f, "-dpng", sprintf ("-r%d", dpi), png);
        minor_grid_undo (undo);
        set (f, "__modified__", false);
        shape.(key) = now_shape;
      endif

      figs{end+1} = describe (f, num, png, W, H);
    catch
      ## A figure that will not print is not worth taking the session's panel down for.
    end_try_catch
  endfor
endfunction

function s = fingerprint (f)
  ## What would make a figure look different, in the cheapest terms that catch it.
  ##
  ## The figure's own size, and for each axes: the limits, the view, where it sits, how
  ## many things are drawn in it, and its title. Between them these cover what the
  ## navigation keys do (limits and view), what a replot does (the child count and
  ## usually the limits) and what labelling does. It is a handful of `get` calls per
  ## figure per command — next to nothing beside the 37 ms a print costs, which is the
  ## whole reason for asking the question at all.
  ##
  ## Not a hash of the PNG: that would mean printing it to find out whether to print it.

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
      parts{end+1} = sprintf ("%g,", get (a, "xlim"), get (a, "ylim"), get (a, "zlim"), ...
                              get (a, "view"), get (a, "position"), numel (get (a, "children")));
      parts{end+1} = t;
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

function undo = minor_grid (f)
  ## MINOR_GRID  Draw the minor grid gnuplot will not, and hand back how to take it away.
  ##
  ## `grid minor` under the gnuplot toolkit prints a figure with no grid at all — not a
  ## faint one, none — and that is the picture a session over ssh gets, because a machine
  ## with no display is exactly where cleecode_boot has to choose gnuplot.
  ##
  ## It is not a setting anyone can fix from Octave. Traced through the command stream
  ## Octave writes to gnuplot (2026-08-22, Octave 11.3.0): the backend asks for the grid
  ## properly — `set mxtics 5; set grid mxtics; set grid linestyle 0, linestyle 10` — and
  ## then, two lines later, states the major tics as an explicit list:
  ##
  ##     set xtics in scale 1.4 border mirror ( "-1" -1.000000, "-0.5" -0.500000, ... );
  ##
  ## gnuplot cannot place minor tics between tics it was handed one by one: there is no
  ## interval for it to subdivide, so it drops mxtics on the floor. Confirmed against
  ## gnuplot directly with two scripts differing in that line alone — `set xtics 0.5` draws
  ## the dotted grid, the list draws nothing. And __gnuplot_draw_axes__.m has exactly one
  ## way of emitting tics (do_tics_1, ~line 2313) and it is the list, always. So
  ## minorgridalpha, xminortick and the rest were all tried and none of them can matter.
  ##
  ## Hence: place the lines ourselves at the positions Octave would have used, print, and
  ## delete them. Only under gnuplot — qt draws its own and a second set on top of it would
  ## be a bug rather than a fix.
  ##
  ## What is deliberately *not* done here: the major grid, which gnuplot does draw, and
  ## 3-D axes, where a grid belongs on three planes and a flat one would be wrong. Both
  ## are left to the toolkit rather than half-corrected.

  undo = struct ("ax", {}, "lines", {}, "xlimmode", {}, "ylimmode", {}, "kids", {});
  try
    if (! strcmp (get (f, "__graphics_toolkit__"), "gnuplot"))
      return;
    endif
  catch
    return;                             # a figure that will not say is left alone
  end_try_catch

  for ax = get (f, "children")'
    try
      if (! strcmp (get (ax, "type"), "axes"))
        continue;
      endif
      ## A legend and a colourbar are axes too, and neither wants a grid inside it.
      if (any (strcmp (get (ax, "tag"), {"legend", "colorbar"})))
        continue;
      endif
      v = get (ax, "view");             # the same "is this 2-D" question describe() asks
      if (! (numel (v) == 2 && abs (v(1)) < 1e-9 && abs (v(2) - 90) < 1e-9))
        continue;
      endif

      wants_x = strcmp (get (ax, "xminorgrid"), "on");
      wants_y = strcmp (get (ax, "yminorgrid"), "on");
      if (! wants_x && ! wants_y)
        continue;
      endif

      xl = get (ax, "xlim");
      yl = get (ax, "ylim");

      ## The colour has to be mixed by hand. qt draws the minor grid at minorgridalpha
      ## over the axes background — 0.25 of a near-black on white, which is the light grey
      ## in the screenshots — and a line object has no alpha to give gnuplot. Blending
      ## against the background gets the same pixels by a different route.
      col = get (ax, "minorgridcolor");
      try
        col = get (ax, "minorgridalpha") * col ...
              + (1 - get (ax, "minorgridalpha")) * get (ax, "color");
      catch
        ## An axes with no alpha or a "none" background: the unblended colour will do.
      end_try_catch
      sty = get (ax, "minorgridlinestyle");
      lw  = get (ax, "linewidth");

      ## Frozen while the lines are in, or the axes rescales around them and the printed
      ## picture is of different limits than the one the panel was told about — measured:
      ## an axes on [-0.8 0.8] came back as [-1 1] the first time this was tried.
      old_x = get (ax, "xlimmode");
      old_y = get (ax, "ylimmode");
      old_kids = get (ax, "children");
      set (ax, "xlimmode", "manual", "ylimmode", "manual");

      ## One object per direction, not one per line. A NaN breaks a line where it stands,
      ## so thirty grid lines travel as a single polyline that lifts its pen thirty times
      ## and the axes gains two children rather than sixty. Measured on the figure from
      ## the screenshots: 112 ms the naive way against 15 ms this way, beside a gnuplot
      ## print that costs about 140 ms on its own. A grid is not worth a prompt that
      ## hesitates, and at a tenth of the print it is not one.
      added = [];
      if (wants_x)
        v = minor_at (get (ax, "xtick"), xl, get (ax, "xscale"));
        if (! isempty (v))
          added(end+1) = line ("parent", ax, ...
                               "xdata", reshape ([v; v; NaN(1, numel (v))], 1, []), ...
                               "ydata", repmat ([yl(1) yl(2) NaN], 1, numel (v)), ...
                               "color", col, "linestyle", sty, "linewidth", lw);
        endif
      endif
      if (wants_y)
        v = minor_at (get (ax, "ytick"), yl, get (ax, "yscale"));
        if (! isempty (v))
          added(end+1) = line ("parent", ax, ...
                               "xdata", repmat ([xl(1) xl(2) NaN], 1, numel (v)), ...
                               "ydata", reshape ([v; v; NaN(1, numel (v))], 1, []), ...
                               "color", col, "linestyle", sty, "linewidth", lw);
        endif
      endif
      if (isempty (added))
        set (ax, "xlimmode", old_x, "ylimmode", old_y);
        continue;
      endif

      ## Behind the data, where a grid goes. children(1) is drawn last, so ours go to the
      ## end of the list rather than the front they arrived at.
      set (ax, "children", [old_kids(:); added(:)]);

      undo(end+1) = struct ("ax", ax, "lines", added, "xlimmode", old_x, ...
                            "ylimmode", old_y, "kids", old_kids);
    catch
      ## An axes that will not take a grid still gets printed without one.
    end_try_catch
  endfor
endfunction

function minor_grid_undo (undo)
  ## Put the axes back exactly as it was found, including the order of its children: the
  ## session is allowed to notice nothing at all.
  for k = 1:numel (undo)
    try
      delete (undo(k).lines);
      set (undo(k).ax, "children", undo(k).kids);
      set (undo(k).ax, "xlimmode", undo(k).xlimmode, "ylimmode", undo(k).ylimmode);
    catch
      ## Better a stray grid line than a session taken down while tidying up.
    end_try_catch
  endfor
endfunction

function p = minor_at (t, lim, scale)
  ## Where Octave itself would have put the minor tics: five divisions per interval on a
  ## linear axis, the 2..9 of each decade on a log one. Both numbers are the backend's own
  ## (__gnuplot_draw_axes__.m: num_mtics = 5, and 10 for log).

  p = [];
  if (strcmp (scale, "log"))
    if (lim(1) <= 0)
      return;
    endif
    for e = floor (log10 (lim(1))) : ceil (log10 (lim(2)))
      p = [p, (2:9) * 10^e];
    endfor
    p = p(p > lim(1) & p < lim(2));
    return;
  endif

  if (numel (t) < 2)
    return;                             # nothing to subdivide
  endif
  step = (t(2) - t(1)) / 5;
  if (! (step > 0) || ! isfinite (step))
    return;
  endif
  ## From below the first major tick to above the last, so the margins either side of the
  ## data get their lines too — gnuplot's own grid covers the whole axes, not just the
  ## span between the outermost tics.
  p = (t(1) - ceil ((t(1) - lim(1)) / step) * step) : step : lim(2);
  p = p(p > lim(1) & p < lim(2));
  ## And not on top of a major tick, where the grid line either already is or is not
  ## wanted. A thousandth of a step is far below anything a tick list resolves.
  for k = 1:numel (t)
    p = p(abs (p - t(k)) > step/1000);
  endfor
endfunction
