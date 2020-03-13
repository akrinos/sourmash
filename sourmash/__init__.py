#! /usr/bin/env python
"""
An implementation of a MinHash bottom sketch, applied to k-mers in DNA.
"""
from __future__ import print_function

from ._lowlevel import ffi, lib

ffi.init_once(lib.sourmash_init, "init")

from pkg_resources import get_distribution, DistributionNotFound

try:
    VERSION = get_distribution(__name__).version
except DistributionNotFound:  # pragma: no cover
    try:
        from .version import version as VERSION  # noqa
    except ImportError:  # pragma: no cover
        raise ImportError(
            "Failed to find (autogenerated) version.py. "
            "This might be because you are installing from GitHub's tarballs, "
            "use the PyPI ones."
        )

from ._minhash import MinHash, get_minhash_default_seed, get_minhash_max_hash

DEFAULT_SEED = get_minhash_default_seed()
MAX_HASH = get_minhash_max_hash()

from .signature import (
    load_signatures,
    load_one_signature,
    SourmashSignature,
    save_signatures,
)

from .sbtmh import load_sbt_index, search_sbt_index, create_sbt_index
from . import lca
from . import sbt
from . import sbtmh
from . import sbt_storage
from . import signature
from . import sig
from . import cli
from . import commands
from . import hash2kmer
