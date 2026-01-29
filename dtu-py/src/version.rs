use pyo3::prelude::*;

const COMMIT_HASH: &'static str = include!(concat!(env!("OUT_DIR"), "/commit_hash"));

#[pyclass]
pub struct Version {
    dtu: dtu::Version,
    commit: &'static str,
}

impl Version {
    pub const fn current() -> Self {
        Self {
            dtu: dtu::VERSION,
            commit: COMMIT_HASH,
        }
    }
}

#[pymethods]
impl Version {
    fn __str__(&self) -> String {
        format!("{} rev {}", self.dtu.to_string(), self.commit)
    }

    fn __repr__(&self) -> String {
        self.__str__()
    }

    #[getter]
    fn major(&self) -> usize {
        self.dtu.major
    }

    #[getter]
    fn minor(&self) -> usize {
        self.dtu.minor
    }

    #[getter]
    fn patch(&self) -> usize {
        self.dtu.patch
    }

    #[getter]
    fn extra(&self) -> Option<&'static str> {
        self.dtu.extra
    }

    #[getter]
    fn commit(&self) -> &'static str {
        self.commit
    }
}
