import hashlib
import pickle
from typing import Optional, List

from dtu import Context, GraphDB, MethodCallPath, ClassSpec, ClassName, MethodSpec


def _key(*args, **kwargs):
    s = "".join(str(e) for e in args) + "".join(str(e) for e in kwargs.values())
    return hashlib.md5(s.encode()).hexdigest()


class CachingGraphDB:
    """
    GraphDB implementation that caches all function call results in the project
    cache directory as pickles
    """

    def __init__(
        self, *, ctx: Optional[Context] = None, graphdb: Optional[GraphDB] = None
    ):
        ctx = ctx if ctx is not None else Context()
        self.ctx = ctx
        self.base = ctx.get_project_cache_dir() / "py-graph-db"
        if not self.base.exists():
            self.base.mkdir(mode=0o700)
        self.wrapped = graphdb if graphdb is not None else GraphDB(self.ctx)

    def _maybe_cached(self, func, *args, **kwargs):
        key = _key(*args, **kwargs)
        cache_file = self.base / f"{key}.pickle"
        if cache_file.exists():
            with cache_file.open("rb") as f:
                return pickle.load(f)
        res = func(*args, **kwargs)
        with cache_file.open("wb") as f:
            pickle.dump(res, f)
        return res

    def find_callers(
        self,
        /,
        *,
        class_=None,
        name=None,
        signature=None,
        method_source=None,
        call_source=None,
        depth=5,
    ) -> List[MethodCallPath]:
        """
        Find all callers of the given class up to a certain depth.

        At least one of `class_` or `name` is required for this search. High depth values may
        negatively impact performance.
        """
        return self._maybe_cached(
            self.wrapped.find_callers,
            class_=class_,
            name=name,
            signature=signature,
            method_source=method_source,
            call_source=call_source,
            depth=depth,
        )

    def find_classes_implementing(
        self, /, iface, *, iface_source=None, impl_source=None
    ) -> List[ClassSpec]:
        """
        Find all classes that implement the given interface
        """
        return self._maybe_cached(
            self.wrapped.find_classes_implementing,
            iface,
            iface_source=iface_source,
            impl_source=impl_source,
        )

    def find_outgoing_calls(
        self, /, *, class_=None, name=None, signature=None, source=None, depth=5
    ) -> List[MethodCallPath]:
        """
        Find all calls leaving the given method up to a given depth.
        """
        return self._maybe_cached(
            self.wrapped.find_outgoing_calls,
            class_=class_,
            name=name,
            signature=signature,
            source=source,
            depth=depth,
        )

    def find_classes_with_method(
        self, name, *, args=None, source=None
    ) -> List[ClassSpec]:
        """
        Find all classes defining the specified method
        """
        return self._maybe_cached(
            self.wrapped.find_classes_with_method, name, args=args, source=source
        )

    def get_all_sources(self, /) -> List[str]:
        """
        Get a set of all sources in the database
        """
        return self._maybe_cached(self.wrapped.get_all_sources)

    def get_classes_for(self, /, src) -> List[ClassName]:
        """
        Get all classes defined by the given source
        """
        return self._maybe_cached(self.wrapped.get_classes_for, src)

    def get_methods_for(self, /, source) -> List[MethodSpec]:
        """
        Get all methods defined by the given soruce
        """
        return self._maybe_cached(self.wrapped.get_methods_for, source)
