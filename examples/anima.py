"""Un'animazione dentro CleeCode — Python.

Il gemello di anima.m. Aprilo e premi ▶ Run: se un prompt Python è già aperto
il file gira lì, quindi alla fine le figure e le variabili restano nella
sessione e ci puoi continuare a lavorare.

La cosa da sapere, ed è il motivo per cui c'è `_cleecode_pyws.frame()` nel
ciclo: le schede si aggiornano quando un'istruzione **finisce**. Un ciclo è
una sola istruzione, quindi senza quella riga la scheda resta ferma al
fotogramma di prima e si muove solo alla fine. `frame()` ristampa le figure
lì, in quel punto del ciclo.

`_cleecode_pyws` è l'unico nome che CleeCode lascia nel tuo namespace, ed è
underscore apposta: `dir()` e il pannello delle variabili restano tuoi.

Fuori da CleeCode non fa niente — non c'è nessuna cartella in cui scrivere —
quindi lo script continua a funzionare dovunque, e la riga qui sotto lo rende
vero anche se `_cleecode_pyws` non esiste affatto.

Quanto va veloce, misurato su questa macchina: matplotlib stampa un
fotogramma di una linea in circa 12 ms, cioè molto più in fretta di quanto
un terminale riesca a disegnarlo. Via ssh il collo di bottiglia è il
collegamento: arriva un PNG per fotogramma, non un flusso video.
"""

# %% controllo
import sys
from pathlib import Path

print(f"interprete: {Path(sys.executable)}")
try:
    import matplotlib
except ModuleNotFoundError:
    raise SystemExit(
        "matplotlib non è in questo python. Installalo qui:\n"
        f"    {sys.executable} -m pip install matplotlib numpy"
    )

import numpy as np
import matplotlib.pyplot as plt

# Fuori da CleeCode questo nome non c'è, e l'animazione deve funzionare lo stesso.
try:
    import cleecode_pyws as _cc
    frame = _cc.frame
except (ImportError, AttributeError):
    def frame():
        """Fuori da CleeCode non c'è niente da aggiornare."""


# %% onda che scorre
# Il caso base: un solo oggetto che cambia dati. I limiti sono fissati, così
# l'animazione non fa ballare anche la cornice.
x = np.linspace(0, 2 * np.pi, 300)
fig1, ax = plt.subplots()
(linea,) = ax.plot(x, np.sin(x), linewidth=2)
ax.set_ylim(-1.2, 1.2)
ax.grid(True)
ax.set_xlabel("x")
ax.set_ylabel("sin(x − t)")
ax.set_title("onda che scorre")
plt.show()

for k in range(120):
    linea.set_ydata(np.sin(x - k / 12))
    frame()                      # senza questa riga la scheda si muove solo alla fine

# %% una superficie che respira
xx, yy = np.meshgrid(np.linspace(-3, 3, 50), np.linspace(-3, 3, 50))
base = (3 * (1 - xx) ** 2 * np.exp(-(xx ** 2) - (yy + 1) ** 2)
        - 10 * (xx / 5 - xx ** 3 - yy ** 5) * np.exp(-(xx ** 2) - yy ** 2)
        - np.exp(-((xx + 1) ** 2) - yy ** 2) / 3)

fig2 = plt.figure()
superficie = fig2.add_subplot(projection="3d")
superficie.set_zlim(-10, 10)
superficie.set_title("respiro")
disegno = superficie.plot_surface(xx, yy, base, cmap="viridis", linewidth=0)
plt.show()

for k in range(60):
    # Una superficie non ha un set_zdata: si ridisegna, togliendo la precedente.
    disegno.remove()
    disegno = superficie.plot_surface(
        xx, yy, base * (0.4 + 0.6 * np.sin(k / 8)), cmap="viridis", linewidth=0
    )
    frame()

# %% un giro attorno alla superficie
# Ruotare è cambiare il punto di vista, e il ciclo è identico. Le stesse frecce
# sulla scheda della figura fanno questo a mano, 15° per volta.
for az in range(-180, 181, 4):
    superficie.view_init(elev=30, azim=az)
    frame()
