"""Optional bundled native RefPack backend; source-only tools retain Python."""
import ctypes
from pathlib import Path

_library=None
_here=Path(__file__).resolve().parent
_native=_here.parents[1]/'target'/'native'
for path in (
    _here/'refpack.dll', _native/'refpack.dll',
    _here/'librefpack.so', _native/'librefpack.so',
    _here/'refpack.so', _native/'refpack.so',
):
    if not path.is_file():
        continue
    try:
        _library=ctypes.CDLL(str(path))
    except OSError:
        continue
    _library.skate_refpack.argtypes=[ctypes.c_char_p,ctypes.c_size_t,ctypes.c_void_p,ctypes.c_size_t,ctypes.c_size_t,ctypes.c_bool]
    _library.skate_refpack.restype=ctypes.c_int
    break

def decode(data,size,start,early=False):
    if _library is None:return None
    output=ctypes.create_string_buffer(size)
    if _library.skate_refpack(data,len(data),output,size,start,early):
        raise ValueError('Malformed RefPack command or declared output size')
    return output.raw
