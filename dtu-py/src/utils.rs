use pyo3::{prelude::*, types::PyTuple, IntoPyObjectExt, PyTypeInfo};
use serde::{de::DeserializeOwned, Serialize};

use crate::exception::DtuError;


pub fn unpickle<T, U>(value: &[u8]) -> PyResult<U>
where
    T: DeserializeOwned,
    U: From<T>,
{
    Ok(U::from(
        postcard::from_bytes(value).map_err(DtuError::mapper)?,
    ))
}

pub fn reduce<'py, T, U>(val: &T, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>>
where
    T: AsRef<U> + PyTypeInfo,
    U: Serialize,
{
    let ty = py.get_type::<T>();
    let data = ::postcard::to_allocvec(val.as_ref()).map_err(DtuError::mapper)?;
    let callable = ty.getattr("__unpickle")?.into_py_any(py)?;
    let ser = ::pyo3::types::PyBytes::new(py, &data).into_py_any(py)?;
    let value = (ser,).into_py_any(py)?;
    Ok(PyTuple::new(py, [callable, value])?)
}
