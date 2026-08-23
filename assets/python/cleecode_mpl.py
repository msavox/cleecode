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

import atexit

from matplotlib.backend_bases import _Backend, FigureManagerBase
from matplotlib.backends.backend_agg import FigureCanvasAgg as FigureCanvas  # noqa: F401


def _hand_over():
    """A script's last chance to give CleeCode its figures.

    Registered here and not in the hook itself, and the reason is atexit's order. Handlers run
    last-registered-first, and `matplotlib.pyplot` registers `Gcf.destroy_all` while it is being
    imported — so anything registered before matplotlib was ever used runs *after* every figure
    has already been destroyed and finds nothing to hand over. That is exactly what a hook set
    up from `sitecustomize` did: measured, `plt.get_fignums()` returns `[]` there every time.
    This module is imported when matplotlib picks its backend, which is necessarily later, so
    this runs first — while the figures are still alive.

    A session that is still at a prompt has already published its figures at every prompt; this
    costs it one more snapshot on the way out. A Python that never plots never imports this
    module and pays nothing at all.
    """
    try:
        import cleecode_pyws

        cleecode_pyws.capture_now()
    except Exception:  # noqa: BLE001 — an interpreter shutting down is not a place to raise
        pass


atexit.register(_hand_over)


@_Backend.export
class _CleeCodeBackend(_Backend):
    FigureCanvas = FigureCanvas
    FigureManager = FigureManagerBase

    @staticmethod
    def show(*args, **kwargs):
        """Hand over without blocking and without drawing anything of its own."""
        return
