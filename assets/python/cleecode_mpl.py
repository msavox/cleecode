"""A matplotlib backend that draws into the session instead of opening a window.

matplotlib's default backend on a desktop puts up a window of its own, which is the thing the
figure panel exists to avoid — and worse, `plt.show()` on a blocking backend does not return
until the window is closed, so a session that plots stops answering until the user notices.

This is Agg with the window taken off. Figures are rendered in memory and the workspace hook
writes them out at the next prompt, the same way it writes everything else. So `show()` has
nothing left to do: it returns immediately, and the figure is already on its way.

It deliberately prints nothing. The first version of this said "[cleecode] figure 1 -> ..." on
every show, which put a line the user did not write into the user's own transcript — the one
thing this design does not do anywhere else.
"""

from matplotlib.backend_bases import _Backend, FigureManagerBase
from matplotlib.backends.backend_agg import FigureCanvasAgg as FigureCanvas  # noqa: F401


@_Backend.export
class _CleeCodeBackend(_Backend):
    FigureCanvas = FigureCanvas
    FigureManager = FigureManagerBase

    @staticmethod
    def show(*args, **kwargs):
        """Hand over without blocking and without drawing anything of its own."""
        return
