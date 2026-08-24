# PYTHONSTARTUP: due righe, e l'unico nome che resta in __main__ e' underscore-prefixed
import cleecode_pyws as _cleecode_pyws
# Dove vanno i grafici, riletto adesso: la variabile d'ambiente e' quella con cui e' partita la
# shell, e la preferenza puo' essere cambiata dopo. Vedi cleecode_pyws.sync_plots.
_cleecode_pyws.sync_plots()
_cleecode_pyws.install()
