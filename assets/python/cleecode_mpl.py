"""Backend matplotlib minimo: plt.show() consegna i PNG a CleeCode invece di aprire finestre."""
import os
from matplotlib.backends.backend_agg import FigureCanvasAgg as FigureCanvas  # noqa: F401
from matplotlib.backend_bases import _Backend, FigureManagerBase

OUT = os.environ.get("CLEECODE_PY_FIGS", ".")

@_Backend.export
class _CleeCodeBackend(_Backend):
    FigureCanvas = FigureCanvas
    FigureManager = FigureManagerBase

    @staticmethod
    def show(*args, **kwargs):
        import matplotlib.pyplot as plt
        os.makedirs(OUT, exist_ok=True)
        for num in plt.get_fignums():
            fig = plt.figure(num)
            path = os.path.join(OUT, f"shown{num}.png")
            fig.savefig(path, dpi=fig.dpi)
            print(f"[cleecode] figura {num} -> {path}")
