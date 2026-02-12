use dtu::utils::smali::SmaliMethodSignatureIterator;
use std::path::PathBuf;

use dtu::utils::{find_files_for_class, find_smali_file_for_class};
use dtu::Context;
use pyo3::prelude::*;

use crate::context::PyContext;
use crate::exception::DtuError;
use crate::types::{PyClassName, PyDevicePath};

#[pyfunction]
pub fn split_smali_args(args: &str) -> PyResult<Vec<String>> {
    let it =
        SmaliMethodSignatureIterator::new(args).map_err(|e| DtuError::new_err(String::from(e)))?;
    Ok(it
        .into_iter()
        .map(|it| it.to_string())
        .collect::<Vec<String>>())
}

#[pyfunction(signature = (class_name, *, ctx=None))]
pub fn find_smali_files_for_class(
    class_name: &PyClassName,
    ctx: Option<&PyContext>,
) -> PyResult<Vec<PathBuf>> {
    let ctx: &dyn Context = match ctx {
        None => &dtu::DefaultContext::new(),
        Some(c) => c,
    };
    Ok(find_files_for_class(ctx, class_name.as_ref()))
}

#[pyfunction(signature = (class_name, *, apk_path=None, ctx=None))]
pub fn get_smali_file_for_class(
    class_name: &PyClassName,
    apk_path: Option<&PyDevicePath>,
    ctx: Option<&PyContext>,
) -> PyResult<Option<PathBuf>> {
    let ctx: &dyn Context = match ctx {
        None => &dtu::DefaultContext::new(),
        Some(c) => c,
    };

    match find_smali_file_for_class(
        ctx,
        class_name.as_ref(),
        apk_path.as_ref().map(|dp| dp.as_ref()),
    ) {
        Some(p) => Ok(p.exists().then_some(p)),
        None => return Ok(None),
    }
}
