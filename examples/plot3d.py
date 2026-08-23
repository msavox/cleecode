"""Prova della rotazione 3-D — Python.

Il gemello di plot3d.m. Aprilo e premi ▶ Run: se un prompt Python è già aperto
in un terminale, il file gira *lì* e le figure restano nella sessione — poi puoi
continuare a scrivere comandi che le usano. Altrimenti CleeCode ne apre una.

Con la scheda della figura davanti:

    ← →   girano attorno all'asse verticale (azimut), 15° per volta
    ↑ ↓   alzano e abbassano il punto di vista (elevazione)
    + −   avvicinano e allontanano
    r     rimette la vista di partenza
    e     esporta la figura in un file

Sono gli stessi tasti che su un grafico piatto lo spostano: CleeCode guarda se
l'asse è tridimensionale e decide di conseguenza, e la barra in fondo alla
scheda dice quale dei due sta facendo. Il comando va alla sessione, che
ridisegna — quindi le etichette degli assi restano vere.

Serve matplotlib **nello stesso python del terminale**: la prima cella lo dice
invece di lasciartelo scoprire da un errore più avanti.
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
print(f"matplotlib: {matplotlib.__version__} (backend {matplotlib.get_backend()})")

import numpy as np
import matplotlib.pyplot as plt

# %% una superficie
# Lo stesso picco dell'esempio Octave: una cima sola e pendenze molto diverse,
# così girandolo si capisce subito da che parte lo stai guardando.
x, y = np.meshgrid(np.linspace(-3, 3, 60), np.linspace(-3, 3, 60))
z = (3 * (1 - x) ** 2 * np.exp(-(x ** 2) - (y + 1) ** 2)
     - 10 * (x / 5 - x ** 3 - y ** 5) * np.exp(-(x ** 2) - y ** 2)
     - np.exp(-((x + 1) ** 2) - y ** 2) / 3)

fig1 = plt.figure()
superficie = fig1.add_subplot(projection="3d")
faccia = superficie.plot_surface(x, y, z, cmap="viridis", linewidth=0)
fig1.colorbar(faccia, shrink=0.6)
superficie.set_xlabel("x")
superficie.set_ylabel("y")
superficie.set_zlabel("z")
superficie.set_title("superficie — prova le frecce")
plt.show()

# %% una curva nello spazio
# Una spirale: di fronte sembra un cerchio, di lato si vede che sale.
t = np.linspace(0, 8 * np.pi, 600)
fig2 = plt.figure()
spirale = fig2.add_subplot(projection="3d")
spirale.plot(np.cos(t), np.sin(t), t, linewidth=2)
spirale.grid(True)
spirale.set_xlabel("cos t")
spirale.set_ylabel("sin t")
spirale.set_zlabel("t")
spirale.set_title("spirale — di fronte sembra un cerchio")
plt.show()

# %% da dove la stiamo guardando
# Premi qualche freccia sulla scheda della figura e rimanda questa cella: i due
# numeri sono cambiati, perché a girare è stata la sessione e non l'immagine.
print(f"elevazione {spirale.elev:.1f}, azimut {spirale.azim:.1f}")
