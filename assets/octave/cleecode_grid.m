function undo = cleecode_grid (f)
  ## CLEECODE_GRID  Draw the grid the way qt would, under gnuplot, and say how to undo it.
  ##
  ## Only under gnuplot. qt draws its own and a second set on top of it would be a bug
  ## rather than a fix.
  ##
  ## Two separate faults are corrected here, and they were found a release apart.
  ##
  ## THE MINOR GRID IS NOT DRAWN AT ALL. `grid minor` under gnuplot prints a figure with no
  ## minor grid — not a faint one, none — and that is the picture a session over ssh gets,
  ## because a machine with no display is exactly where cleecode_boot has to choose
  ## gnuplot. It is not a setting anyone can fix from Octave. Traced through the command
  ## stream Octave writes to gnuplot (2026-08-22, Octave 11.3.0): the backend asks for the
  ## grid properly — `set mxtics 5; set grid mxtics; set grid linestyle 0, linestyle 10` —
  ## and then, two lines later, states the major tics as an explicit list:
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
  ## THE MAJOR GRID IS DRAWN, AND DRAWN WRONG. This one was left to gnuplot on purpose the
  ## first time round, on the grounds that at least it appears. Put the two prints of the
  ## same figure side by side and that is plainly the wrong call: qt draws the major grid
  ## as `gridcolor` at `gridalpha`, which is 15% of a near-black over white — a grey you
  ## read the plot *through*. gnuplot draws it as a solid near-black line at full strength,
  ## and it is the loudest thing in the picture: the data is a thin blue curve behind a
  ## black cage. Neither the colour nor the alpha reaches it, for the same reason as above
  ## — they are properties of a grid gnuplot is drawing from its own defaults.
  ##
  ## Correcting one and not the other was the worst of the three states: two greys and two
  ## weights in one picture, which reads as a mistake even to somebody who has never seen
  ## the qt version. So both are ours now, and gnuplot's own grid is switched off for the
  ## print rather than drawn under them.
  ##
  ## What is still deliberately not done: 3-D axes, where a grid belongs on three planes
  ## and a flat one would be wrong. Left to the toolkit rather than half-corrected.
  ##
  ## Its own file rather than a subfunction of cleecode_figs so it can be called, printed
  ## and looked at on its own — which is how the second fault above was found at all.

  undo = struct ("ax", {}, "lines", {}, "xlimmode", {}, "ylimmode", {}, "kids", {}, ...
                 "xgrid", {}, "ygrid", {});
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

      wants = struct ("x",  strcmp (get (ax, "xgrid"), "on"), ...
                      "y",  strcmp (get (ax, "ygrid"), "on"), ...
                      "mx", strcmp (get (ax, "xminorgrid"), "on"), ...
                      "my", strcmp (get (ax, "yminorgrid"), "on"));
      if (! (wants.x || wants.y || wants.mx || wants.my))
        continue;
      endif

      xl = get (ax, "xlim");
      yl = get (ax, "ylim");

      ## The colours have to be mixed by hand. qt draws each grid at its alpha over the
      ## axes background — 15% of a near-black on white for the major one, 25% for the
      ## minor — and a line object has no alpha to give gnuplot. Blending against the
      ## background gets the same pixels by a different route.
      major = blend (ax, "gridcolor", "gridalpha");
      minor = blend (ax, "minorgridcolor", "minorgridalpha");
      lw = get (ax, "linewidth");

      ## Frozen while the lines are in, or the axes rescales around them and the printed
      ## picture is of different limits than the one the panel was told about — measured:
      ## an axes on [-0.8 0.8] came back as [-1 1] the first time this was tried.
      old_x = get (ax, "xlimmode");
      old_y = get (ax, "ylimmode");
      old_kids = get (ax, "children");
      old_xgrid = get (ax, "xgrid");
      old_ygrid = get (ax, "ygrid");
      set (ax, "xlimmode", "manual", "ylimmode", "manual");

      ## One object per direction, not one per line. A NaN breaks a line where it stands,
      ## so thirty grid lines travel as a single polyline that lifts its pen thirty times
      ## and the axes gains two children rather than sixty. Measured on the figure from
      ## the screenshots: 112 ms the naive way against 15 ms this way, beside a gnuplot
      ## print that costs about 140 ms on its own. A grid is not worth a prompt that
      ## hesitates, and at a tenth of the print it is not one.
      added = [];
      ## Minor first, so the major lines end up over them where they cross — which is the
      ## order qt draws them in, and the only place the two are distinguishable at all.
      if (wants.mx)
        added = [added, verticals(ax, minor_at (get (ax, "xtick"), xl, get (ax, "xscale")), ...
                                  yl, minor, get (ax, "minorgridlinestyle"), lw)];
      endif
      if (wants.my)
        added = [added, horizontals(ax, minor_at (get (ax, "ytick"), yl, get (ax, "yscale")), ...
                                    xl, minor, get (ax, "minorgridlinestyle"), lw)];
      endif
      ## The major lines stop one tick short of the frame at each end, and that gap is the
      ## whole reason they look attached to the axis at all.
      ##
      ## qt draws its grid lines the full height and then paints the tick marks over them,
      ## because in qt the axes decoration is in front. gnuplot's is behind: it draws the
      ## border and its tics first and the plot data — which is what our lines are, as far
      ## as it is concerned — on top. So a line running the whole height covers the tick it
      ## is supposed to be standing on, and the result is a grid floating free of an axis
      ## with no tics on it. Which is exactly what it looks like, and exactly what somebody
      ## comparing the two pictures notices first.
      ##
      ## Leaving the tick's worth of room at each end lets gnuplot's own black tic show
      ## through, and the pair reads as qt's does: a dark mark where the light line meets
      ## the axis. `ticklength(1)` is the fraction Octave itself uses for a 2-D axes.
      tl = 0;
      try
        tl = get (ax, "ticklength")(1);
      catch
      end_try_catch
      if (wants.x)
        added = [added, verticals(ax, inside (get (ax, "xtick"), xl), shrink (yl, tl), ...
                                  major, get (ax, "gridlinestyle"), lw)];
      endif
      if (wants.y)
        added = [added, horizontals(ax, inside (get (ax, "ytick"), yl), shrink (xl, tl), ...
                                    major, get (ax, "gridlinestyle"), lw)];
      endif

      if (isempty (added))
        set (ax, "xlimmode", old_x, "ylimmode", old_y);
        continue;
      endif

      ## gnuplot's own grid goes off for the print. Left on it would draw its black cage
      ## underneath ours, which is both of them at once and worse than either.
      set (ax, "xgrid", "off", "ygrid", "off");

      ## Behind the data, where a grid goes. children(1) is drawn last, so ours go to the
      ## end of the list rather than the front they arrived at.
      set (ax, "children", [old_kids(:); added(:)]);

      undo(end+1) = struct ("ax", ax, "lines", added, "xlimmode", old_x, ...
                            "ylimmode", old_y, "kids", old_kids, ...
                            "xgrid", old_xgrid, "ygrid", old_ygrid);
    catch
      ## An axes that will not take a grid still gets printed without one.
    end_try_catch
  endfor
endfunction

function c = blend (ax, colour_prop, alpha_prop)
  ## A grid colour as qt would have composited it: the colour at its alpha over whatever
  ## the axes background is.
  c = get (ax, colour_prop);
  try
    a = get (ax, alpha_prop);
    c = a * c + (1 - a) * get (ax, "color");
  catch
    ## An axes with no alpha or a "none" background: the unblended colour will do.
  end_try_catch
endfunction

function h = verticals (ax, at, lim, col, sty, lw)
  h = [];
  if (isempty (at))
    return;
  endif
  h = line ("parent", ax, ...
            "xdata", reshape ([at(:)'; at(:)'; NaN(1, numel (at))], 1, []), ...
            "ydata", repmat ([lim(1) lim(2) NaN], 1, numel (at)), ...
            "color", col, "linestyle", sty, "linewidth", lw);
endfunction

function h = horizontals (ax, at, lim, col, sty, lw)
  h = [];
  if (isempty (at))
    return;
  endif
  h = line ("parent", ax, ...
            "xdata", repmat ([lim(1) lim(2) NaN], 1, numel (at)), ...
            "ydata", reshape ([at(:)'; at(:)'; NaN(1, numel (at))], 1, []), ...
            "color", col, "linestyle", sty, "linewidth", lw);
endfunction

function lim = shrink (lim, fraction)
  ## The span less a tick's worth at each end. The minor grid does not get this: nothing
  ## draws a minor tic here, so there is nothing at the frame for a minor line to cover.
  d = fraction * (lim(2) - lim(1));
  lim = [lim(1) + d, lim(2) - d];
endfunction

function t = inside (t, lim)
  ## Major tics strictly within the limits. One exactly on a limit is already drawn: it is
  ## the box round the axes, and a grid line on top of it is a second, slightly different
  ## grey over the frame.
  t = t(t > lim(1) & t < lim(2));
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
