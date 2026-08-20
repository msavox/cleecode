function plot_tick (dir)
  persistent seen
  try
    if (isempty (seen)) seen = []; endif
    figs = get (0, "children");
    for f = figs(:)'
      png = fullfile (dir, sprintf ("fig%d.png", f));
      print (f, "-dpng", "-r96", png);
    endfor
    if (! isempty (figs))
      fid = fopen (fullfile (dir, "figs.log"), "a");
      fprintf (fid, "%.3f printed %d fig(s)\n", time (), numel (figs));
      fclose (fid);
    endif
  catch err
    fid = fopen (fullfile (dir, "figs.log"), "a");
    fprintf (fid, "%.3f ERR %s\n", time (), err.message); fclose (fid);
  end_try_catch
endfunction
