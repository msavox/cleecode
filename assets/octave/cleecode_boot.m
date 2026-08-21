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

  ## Where this session's plots go, decided by CleeCode and passed in rather than asked
  ## about here: it knows whether there is a screen to open a window on, and this does not.
  ##
  ## "windows" leaves Octave exactly as it is outside CleeCode — its own toolkit, its own
  ## figure windows, nothing captured. The workspace panel still fills in: variables are not
  ## plots, and nobody choosing where their figures appear asked to stop seeing those.
  if (strcmp (getenv ("CLEECODE_PLOTS"), "windows"))
    ## The tick prints figures to the directory this names; empty, cleecode_figs returns at
    ## its first line. Cleared here rather than left unset by CleeCode so that one variable
    ## holds the decision and this file is the only place that acts on it.
    setenv ("CLEECODE_OCTAVE_FIGS", "");
    cleecode_ws (ws);
    return;
  endif

  ## No figure window ever opens. Reparenting a live Qt window into a terminal is not
  ## possible, so a plot reaches CleeCode as a picture instead — and a window appearing
  ## behind the terminal, which is what happens without this, is the worst of both.
  set (0, "defaultfigurevisible", "off");
  cleecode_toolkit ();
  cleecode_ws (ws);
endfunction

function cleecode_toolkit ()
  ## Prefer a toolkit that can draw with no screen to draw on.
  ##
  ## A session CleeCode drives never shows a window, so its toolkit only has to be able to
  ## *print*. That makes gnuplot the right choice on a machine with no display — a remote
  ## server over ssh, which is exactly where this first went wrong. qt cannot load without a
  ## display, and with gnuplot not installed either Octave has no toolkit at all: the user
  ## then learns about it from inside whichever line of their own script first called
  ## figure(), as "error: no graphics toolkits are available!".
  ##
  ## Measured, both toolkits print the same figure to the same size and the same axes
  ## geometry; gnuplot took 451 ms against qt's 298 ms for a line plot. Slower and working
  ## beats faster and absent.
  try
    have = available_graphics_toolkits ();
    if (isempty (have))
      return;               # nothing to choose from; the tick reports it in the panel
    endif
    ## ismac is asked because a Mac has no DISPLAY and does not need one.
    headless = ! ismac () && isempty (getenv ("DISPLAY")) && isempty (getenv ("WAYLAND_DISPLAY"));
    if (any (strcmp (have, "gnuplot")) && (headless || ! any (strcmp (have, "qt"))))
      ## Octave answers this choice with nine lines about gnuplot being discouraged and the
      ## qt toolkit being recommended instead. It prints them when the toolkit is first
      ## *used*, not when it is chosen — so on a machine whose only toolkit is gnuplot they
      ## do not arrive at startup where a banner belongs, they arrive in the middle of the
      ## user's own output, at their first plot.
      ##
      ## Turned off for the session rather than around the call below, which is where that
      ## lands it. The advice is about a choice CleeCode made on the user's behalf, and what
      ## it recommends — use qt — is not available on a machine with no display. This one
      ## identifier and nothing else: the session's other warnings are the user's.
      warning ("off", "Octave:gnuplot-graphics");
      graphics_toolkit ("gnuplot");
    endif
  catch
    ## A session that cannot choose a toolkit still gets its workspace panel.
  end_try_catch
endfunction
