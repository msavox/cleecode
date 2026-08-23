function cleecode_grid_undo (undo)
  ## CLEECODE_GRID_UNDO  Put back what cleecode_grid changed, exactly as it was found.
  ##
  ## Including the order of the axes' children and whether gnuplot's own grid was on: the
  ## session is allowed to notice nothing at all.

  for k = 1:numel (undo)
    try
      delete (undo(k).lines);
      set (undo(k).ax, "children", undo(k).kids);
      set (undo(k).ax, "xgrid", undo(k).xgrid, "ygrid", undo(k).ygrid);
      set (undo(k).ax, "xlimmode", undo(k).xlimmode, "ylimmode", undo(k).ylimmode);
    catch
      ## Better a stray grid line than a session taken down while tidying up.
    end_try_catch
  endfor
endfunction
