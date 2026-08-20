function wsinfo (pat)
  ## WSINFO  Workspace "esteso": nome, classe, size, min, max, mean/preview.
  ##   wsinfo        elenca tutte le variabili dello scope chiamante
  ##   wsinfo ("a*") solo quelle che matchano il pattern

  if (nargin < 1)
    w = evalin ("caller", "whos");
  else
    w = evalin ("caller", sprintf ("whos ('%s')", pat));
  endif

  if (isempty (w))
    disp ("workspace vuoto");
    return;
  endif

  printf ("%-16s %-16s %-10s %12s %12s  %s\n", ...
          "Name", "Class", "Size", "Min", "Max", "Mean/Info");
  printf ("%s\n", repmat ("-", 1, 84));

  for k = 1:numel (w)
    name = w(k).name;
    val  = evalin ("caller", name);
    sz   = sprintf ("%dx", w(k).size);
    sz   = sz(1:end-1);

    mn = ""; mx = ""; extra = "";

    if (isnumeric (val) || islogical (val))
      if (isempty (val))
        extra = "(empty)";
      elseif (iscomplex (val))
        mn = sprintf ("%.4g", min (abs (val(:))));
        mx = sprintf ("%.4g", max (abs (val(:))));
        extra = "(|z|)";
      else
        v = double (val(:));
        nn = sum (isnan (v));
        v = v(!isnan (v));
        mn = sprintf ("%.4g", min (v));
        mx = sprintf ("%.4g", max (v));
        extra = sprintf ("mean %.4g", mean (v));
        if (nn > 0)
          extra = sprintf ("%s, %d NaN", extra, nn);
        endif
      endif
    elseif (ischar (val))
      txt = val(1,:);
      if (numel (txt) > 24)
        txt = [txt(1:21) "..."];
      endif
      extra = ["'" txt "'"];
    elseif (iscell (val))
      extra = sprintf ("%d elementi", numel (val));
    elseif (isstruct (val))
      f = fieldnames (val);
      extra = sprintf ("campi: %s", strjoin (f(1:min (4, numel (f)))', ","));
    elseif (is_function_handle (val))
      extra = func2str (val);
    endif

    printf ("%-16s %-16s %-10s %12s %12s  %s\n", ...
            name, w(k).class, sz, mn, mx, extra);
  endfor
endfunction
