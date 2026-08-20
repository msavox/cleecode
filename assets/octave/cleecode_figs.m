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
  ## The size is forced through paperposition. A figure created with position
  ## [0 0 800 600] does *not* print to an 800x600 PNG: `print` sizes from the paper in
  ## inches and ignores the on-screen pixels, so it comes out at whatever
  ## paper x dpi happens to be. Every mouse coordinate mapped onto the result would be
  ## quietly wrong, which is worse than visibly wrong.

  figs = {};
  if (isempty (dir))
    return;
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

      ## A figure that changed, or one whose PNG is not there — which is how the first
      ## snapshot after CleeCode restarts still has pictures in it.
      if (strcmp (get (f, "__modified__"), "on") || ! exist (png, "file"))
        set (f, "paperunits", "inches", ...
                "paperposition", [0 0 W/dpi H/dpi], ...
                "papersize", [W/dpi H/dpi]);
        print (f, "-dpng", sprintf ("-r%d", dpi), png);
        set (f, "__modified__", false);
      endif

      figs{end+1} = describe (f, num, png, W, H);
    catch
      ## A figure that will not print is not worth taking the session's panel down for.
    end_try_catch
  endfor
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
