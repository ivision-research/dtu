use dtu::{
    db::graph::models::{MethodSearch, MethodSearchParams},
    utils::ClassName,
};

pub fn get_search_params<'a>(
    name: Option<&'a str>,
    class: Option<&'a ClassName>,
    signature: Option<&'a str>,
) -> anyhow::Result<MethodSearchParams<'a>> {
    Ok(MethodSearchParams::new(name, class, signature)
        .map_err(|it| anyhow::Error::msg(format!("failed to get search params: {it}")))?)
}

pub fn get_method_search<'a>(
    name: Option<&'a str>,
    class: Option<&'a ClassName>,
    signature: Option<&'a str>,
    source: Option<&'a str>,
) -> anyhow::Result<MethodSearch<'a>> {
    Ok(MethodSearch::new(
        get_search_params(name, class, signature)?,
        source,
    ))
}
