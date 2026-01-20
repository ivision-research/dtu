use std::io;

use pyo3::{create_exception, PyErr};

create_exception!(dtu, DtuError, pyo3::exceptions::PyException);

pub(crate) struct DtuBaseError(dtu::Error);

impl From<dtu::Error> for DtuBaseError {
    fn from(value: dtu::Error) -> Self {
        Self(value)
    }
}

impl DtuError {
    pub fn mapper<T: ToString>(value: T) -> PyErr {
        DtuError::new_err(value.to_string())
    }
}

impl From<io::Error> for DtuBaseError {
    fn from(value: io::Error) -> Self {
        Self(value.into())
    }
}

impl From<String> for DtuBaseError {
    fn from(value: String) -> Self {
        Self(dtu::Error::Generic(value))
    }
}

impl<'a> From<&'a str> for DtuBaseError {
    fn from(value: &'a str) -> Self {
        Self(dtu::Error::Generic(value.into()))
    }
}

impl From<DtuBaseError> for PyErr {
    fn from(value: DtuBaseError) -> Self {
        DtuError::new_err(value.0.to_string())
    }
}
