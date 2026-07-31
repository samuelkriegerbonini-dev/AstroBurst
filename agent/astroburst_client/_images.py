# AstroBurst Python client — contributed by Jae-Joon Lee <https://github.com/leejjoon>
"""
Optional Pillow integration.

Render endpoints return raw PNG bytes (zero required dependencies).
`to_pillow()` converts those bytes to a PIL.Image when Pillow is installed.
Install the optional extra to enable:

    pip install astroburst-client[image]
"""
from __future__ import annotations

import io
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from PIL import Image as _PilImage


def to_pillow(data: bytes) -> "_PilImage.Image":
    """
    Convert raw PNG bytes to a ``PIL.Image.Image``.

    Requires the ``[image]`` extra (``pip install astroburst-client[image]``).

    Parameters
    ----------
    data:
        Raw PNG bytes as returned by ``Session.stf()``, ``Session.render()``,
        or ``Session.viewport()``.

    Returns
    -------
    PIL.Image.Image

    Raises
    ------
    RuntimeError
        If Pillow is not installed.
    """
    try:
        from PIL import Image
    except ImportError as exc:
        raise RuntimeError(
            "Pillow is required for to_pillow().\n"
            "Install it with:  pip install astroburst-client[image]"
        ) from exc

    return Image.open(io.BytesIO(data))