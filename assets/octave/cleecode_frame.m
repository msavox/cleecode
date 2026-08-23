function cleecode_frame ()
  ## CLEECODE_FRAME  Put what the figures look like right now into their tabs.
  ##
  ## For a loop. Everything else here happens at the prompt: the workspace hook runs from
  ## `add_input_event_hook`, which Octave calls while it is *waiting for input* — measured, it
  ## does not fire once during a command that takes two seconds. That is the right design for a
  ## panel of variables, and it means an animation is invisible while it runs: the tab holds the
  ## frame from before the loop until the loop is over.
  ##
  ## So a loop that wants to be watched says so:
  ##
  ##     for k = 1:200
  ##       set (h, "ydata", sin (x + k/20));
  ##       cleecode_frame ();
  ##     endfor
  ##
  ## Measured on this machine: 28 ms for a line plot and 35 ms for a 60x60 surface, so about
  ## thirty frames a second before the terminal has drawn anything. A `pause` in the loop is
  ## still worth having if the arithmetic between frames is quick — not to let CleeCode catch
  ## up, but so the animation runs at the speed you meant rather than as fast as it can.
  ##
  ## Outside CleeCode, and in a session set to use real figure windows, it does nothing at all:
  ## the directory it would write to is empty, and cleecode_figs returns at its first line. So a
  ## script with this in it still runs anywhere, which is the point of it being a call and not a
  ## setting.

  try
    cleecode_figs (getenv ("CLEECODE_OCTAVE_FIGS"));
  catch
    ## A frame that will not print is not worth stopping somebody's loop for.
  end_try_catch
endfunction
