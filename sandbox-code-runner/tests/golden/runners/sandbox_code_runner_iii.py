# sandbox-code-runner guest iii library — planted at runtime creation. Do not edit
# in place. make_iii() returns the global every eval and handler gets: a
# LAZY handle on the real iii-sdk client (pip-installed at runtime
# creation). Nothing connects until the first attribute access, so code
# that never touches `iii` pays nothing.
#
# NOT named iii.py: `python3 <script>` puts the script's own directory at
# sys.path[0], and a sibling iii.py would shadow the SDK's real `iii`
# package for every eval and handler.
import os


class _LazyIii:
    def __init__(self):
        self._client = None

    def _resolve(self):
        if self._client is not None:
            return self._client
        url = os.environ.get("III_URL")
        if not url:
            raise RuntimeError("iii is unavailable: III_URL is not set for this runtime")
        try:
            from iii import InitOptions, register_worker
        except ModuleNotFoundError as exc:
            raise RuntimeError(
                "iii is unavailable: the iii-sdk package is not installed in this "
                "runtime (its pip install at runtime creation may have failed): "
                f"{exc}"
            ) from exc
        name = os.environ.get("III_WORKER_NAME")
        options = InitOptions(worker_name=name) if name else None
        self._client = register_worker(url, options)
        return self._client

    def __getattr__(self, name):
        return getattr(self._resolve(), name)

    # Introspection never connects: printing the global or dir()-ing it is
    # how an unknown API gets explored, and it must answer usefully with no
    # side effects.
    def __repr__(self):
        if self._client is None:
            return (
                "<iii: lazy iii-sdk client (connects on first use); e.g. "
                "iii.trigger({'function_id': 'worker::fn', 'payload': {}}); "
                "docs: https://iii.dev/docs/reference/sdk-python>"
            )
        return repr(self._client)

    def __dir__(self):
        return [] if self._client is None else dir(self._client)


def make_iii():
    return _LazyIii()
