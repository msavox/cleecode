"""Prova dei plot dal lato Python — l'equivalente di plot.m per pylab.

Si può usare in tre modi, ed è scritto per provarli tutti e tre:

  · il pulsante ▶ Run, che lo esegue dall'inizio alla fine;
  · una cella per volta con Ctrl+Shift+X, mandandola alla sessione già aperta —
    ogni blocco che comincia con `# %%` è una cella;
  · il preset `clee -w pylab`, che apre prompt, pannello delle variabili e
    le figure come schede.

Una cosa da sapere prima, ed è l'unico modo in cui questo può non funzionare:
matplotlib deve stare **nello stesso python che gira nel terminale**. Se il
prompt è un pyenv e matplotlib l'hai installato col python di Homebrew, qui
non si vede — `import matplotlib` fallisce e non è CleeCode a fallire. La
prima cella lo dice invece di lasciartelo scoprire da un errore più avanti.

Dove finiscono le figure lo decidi tu, e vale dalla sessione dopo:
impostazione `plots` — `tabs` le mette come schede, `windows` le lascia alle
finestre vere di matplotlib. Su una macchina senza schermo la scelta non c'è.
"""

# %% controllo
# Chi sta girando davvero, e se ha matplotlib. Se questa cella si lamenta, il
# resto del file non ha modo di funzionare.
import sys
from pathlib import Path

print(f"interprete: {Path(sys.executable)}")
try:
    import matplotlib

    print(f"matplotlib: {matplotlib.__version__} (backend {matplotlib.get_backend()})")
except ModuleNotFoundError:
    raise SystemExit(
        "matplotlib non è in questo python. Installalo qui:\n"
        f"    {sys.executable} -m pip install matplotlib numpy"
    )

import numpy as np
import matplotlib.pyplot as plt

# %% una figura con la griglia
# La griglia maggiore e quella minore insieme: è il caso che sul lato Octave
# ha richiesto del lavoro, perché gnuplot la minore non la disegna e la
# maggiore la disegna nera. matplotlib le disegna da sé e non passa da
# gnuplot, quindi qui non c'è niente da correggere — questa cella serve
# proprio a vedere che è così.
t = np.linspace(0, 10, 400)
segnale = np.sin(t) * np.exp(-t / 12)

fig1, ax = plt.subplots()
ax.plot(t, segnale, linewidth=2, label="sin(t)·e^(−t/12)")
ax.grid(True, which="major")
ax.grid(True, which="minor", alpha=0.4)
ax.minorticks_on()
ax.set_xlabel("t")
ax.set_ylabel("ampiezza")
ax.set_title("smorzata, con griglia maggiore e minore")
ax.legend()
plt.show()

# %% una seconda figura
# Due figure aperte insieme: in modalità schede diventano due tab, e si
# passa dall'una all'altra come fra due file.
angoli = np.linspace(0, 2 * np.pi, 240)
raggio = 1 + 0.4 * np.cos(5 * angoli)

fig2, polare = plt.subplots(subplot_kw={"projection": "polar"})
polare.plot(angoli, raggio, linewidth=2)
polare.set_title("una figura che non è un grafico cartesiano")
plt.show()

# %% ridisegnare quella di prima
# Cambia la figura che è già aperta invece di aprirne una terza. La scheda
# deve aggiornarsi: è il caso che era rotto su ogni macchina con uno schermo
# fino alla 0.10, perché la figura non veniva mai marcata come cambiata.
rumore = 0.08 * np.random.default_rng(7).standard_normal(t.size)
fig1.axes[0].plot(t, segnale + rumore, linewidth=1, alpha=0.7, label="con rumore")
fig1.axes[0].legend()
fig1.canvas.draw_idle()
plt.show()

# %% qualcosa da guardare nel pannello
# Variabili di forme diverse, per il pannello delle variabili e per
# l'ispettore (Ctrl+Shift+I su un nome, o Invio sulla riga del pannello).
matrice = np.outer(np.arange(1, 7), np.arange(1, 7))
vettore = np.linspace(-1, 1, 11)
scalare = float(np.trapezoid(segnale, t))
etichetta = "una stringa, per vedere che il pannello non le tratta come numeri"

print(f"integrale della smorzata: {scalare:.6f}")
print(f"matrice {matrice.shape}, vettore {vettore.shape}")
