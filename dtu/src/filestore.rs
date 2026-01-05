use std::{
    borrow::Cow,
    fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;

use crate::{
    config::{parse_config, ConfigMap},
    run_cmd,
    utils::{ensure_dir_exists, maybe_link, replace_char, OS_PATH_SEP_CHAR, SQUASH_PATH_SEP_CHAR},
    Context,
};

pub trait FileStore: Send + Sync {
    /// Put the file at `local_path` into the store at `remote_path`
    fn put_file(&self, ctx: &dyn Context, local_path: &str, remote_path: &str)
        -> crate::Result<()>;

    /// Retrive the file `remote_path` from the store and write it to `local_path`
    fn get_file(&self, ctx: &dyn Context, remote_path: &str, local_path: &str)
        -> crate::Result<()>;

    /// List files in the given directory.
    fn list_files(&self, ctx: &dyn Context, dir: Option<&str>) -> crate::Result<Vec<String>>;

    /// Remove the given file
    fn remove_file(&self, ctx: &dyn Context, file: &str) -> crate::Result<()>;

    fn name(&self) -> &'static str;
}

fn get_filestore_env(ctx: &dyn Context) -> Option<Box<dyn FileStore>> {
    if let Some(s3_bucket) = ctx.maybe_get_env("DTU_S3_BUCKET") {
        let profile = ctx.maybe_get_env("DTU_S3_PROFILE").unwrap_or_else(|| {
            ctx.maybe_get_env("AWS_PROFILE")
                .unwrap_or_else(|| String::from("dtu"))
        });

        let bin = match ctx.maybe_get_env("DTU_S3_AWS_BIN") {
            Some(v) => v,
            None => ctx.maybe_get_bin("aws")?,
        };

        let store = S3FileStore::new(
            bin,
            s3_bucket,
            profile,
            ctx.has_env("DTU_S3_CACHE"),
            ctx.has_env("DTU_S3_CACHE_IS_LINK"),
        );
        return Some(Box::new(store));
    }

    if let Some(path) = ctx.maybe_get_env("DTU_FILESTORE_PATH") {
        let store = LocalFileStore::new(PathBuf::from(path), ctx.has_env("DTU_FILESTORE_LINK"));
        return Some(Box::new(store));
    }
    None
}

fn get_default_filestore(ctx: &dyn Context) -> crate::Result<Box<dyn FileStore>> {
    let base = Box::new(LocalFileStore::new_from_ctx(ctx)?);
    Ok(base)
}

/// Get the file store based on the configuration file.
///
/// If no file store is configured, default to returning a LocalFileStore
pub fn get_filestore(ctx: &dyn Context) -> crate::Result<Box<dyn FileStore>> {
    // Let the environment override everything
    if let Some(from_env) = get_filestore_env(ctx) {
        return Ok(from_env);
    }

    let config_dir = ctx.get_user_config_dir()?;
    let config_file = config_dir.join("config.toml");
    if !config_file.exists() {
        return get_default_filestore(ctx);
    }

    parse_config(
        &config_file,
        |cfg: &ConfigMap| -> crate::Result<Box<dyn FileStore>> {
            if !cfg.has("filestore") {
                return get_default_filestore(ctx);
            }

            let filestore = match cfg.maybe_get_map_typecheck("filestore")? {
                Some(v) => v,
                None => return get_default_filestore(ctx),
            };

            match filestore.maybe_get_map_typecheck("s3")? {
                Some(v) => Ok(Box::new(S3FileStore::from_config(ctx, &v)?)),
                None => match filestore.maybe_get_map_typecheck("local")? {
                    Some(v) => Ok(Box::new(LocalFileStore::from_config(&v)?)),
                    None => Err(crate::Error::new_cfg(
                        &config_file,
                        "need one of filestore.s3 or filestore.local",
                    )),
                },
            }
        },
    )
}

/// Implementation of a `FileStore` using the local file system
pub struct LocalFileStore {
    base: PathBuf,
    get_is_link: bool,
}

impl LocalFileStore {
    pub const NAME: &'static str = "LocalFileStore";

    pub fn new(base: PathBuf, get_is_link: bool) -> Self {
        Self { base, get_is_link }
    }

    fn new_from_ctx(ctx: &dyn Context) -> crate::Result<Self> {
        let base = ctx.get_user_local_dir()?.join("filestore");
        let get_is_link = false;
        Ok(Self::new(base, get_is_link))
    }

    fn from_config(cfg: &ConfigMap) -> crate::Result<Self> {
        let base_str = cfg.must_get_str("base")?;
        let base = PathBuf::from(base_str);
        let get_is_link = cfg.get_bool_or("get-is-link", false);
        Ok(Self { base, get_is_link })
    }

    fn join_path(&self, path: &str) -> crate::Result<PathBuf> {
        let as_path = Path::new(path);
        if as_path.is_absolute() {
            return Err(crate::Error::Generic(format!(
                "can't add absolute paths to the file store: {}",
                path
            )));
        }

        let joined = self.base.join(as_path);
        if !joined.starts_with(&self.base) {
            return Err(crate::Error::Generic(format!(
                "invalid path for file store: {}",
                path
            )));
        }

        Ok(joined)
    }
}

impl FileStore for LocalFileStore {
    fn name(&self) -> &'static str {
        LocalFileStore::NAME
    }

    fn list_files(&self, _ctx: &dyn Context, dir: Option<&str>) -> crate::Result<Vec<String>> {
        let list_dir = match dir {
            Some(v) => Cow::Owned(self.base.join(v)),
            None => Cow::Borrowed(self.base.as_path()),
        };
        Ok(fs::read_dir(&list_dir)?
            .filter_map(|it| {
                let path = it.ok()?.path();
                Some(String::from(path.to_str()?))
            })
            .collect())
    }

    fn remove_file(&self, _ctx: &dyn Context, file: &str) -> crate::Result<()> {
        let path = self.base.join(file);
        Ok(fs::remove_file(&path)?)
    }

    fn put_file(
        &self,
        _ctx: &dyn Context,
        local_path: &str,
        remote_path: &str,
    ) -> crate::Result<()> {
        let into = self.join_path(remote_path)?;
        fs::copy(local_path, &into)?;
        Ok(())
    }

    fn get_file(
        &self,
        _ctx: &dyn Context,
        remote_path: &str,
        local_path: &str,
    ) -> crate::Result<()> {
        let from = self.join_path(remote_path)?;
        if self.get_is_link {
            maybe_link(remote_path, local_path)?;
        } else {
            fs::copy(&from, local_path)?;
        }
        Ok(())
    }
}

/// Implementation of a `FileStore` backed by S3
pub struct S3FileStore {
    bin: String,
    bucket: String,
    profile: String,
    cache: bool,
    cache_is_link: bool,
}

struct AwsListResult(Vec<String>);

impl Into<Vec<String>> for AwsListResult {
    fn into(self) -> Vec<String> {
        self.0
    }
}

impl<'de> Deserialize<'de> for AwsListResult {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct ListItem {
            #[serde(rename = "Key")]
            key: String,
        }

        #[derive(Deserialize)]
        struct ListResult {
            #[serde(rename = "Contents")]
            contents: Vec<ListItem>,
        }

        let res = ListResult::deserialize(deserializer)?;

        let mut items = Vec::with_capacity(res.contents.len());

        for it in res.contents {
            items.push(it.key);
        }

        Ok(Self(items))
    }
}

impl S3FileStore {
    pub const NAME: &'static str = "S3FileStore";

    pub fn new(
        bin: String,
        bucket: String,
        profile: String,
        cache: bool,
        cache_is_link: bool,
    ) -> Self {
        Self {
            bin,
            bucket,
            profile,
            cache,
            cache_is_link,
        }
    }

    fn from_config(ctx: &dyn Context, cfg: &ConfigMap) -> crate::Result<Self> {
        let bucket = cfg.must_get_str("bucket")?.to_string();
        let profile = match cfg.maybe_get_str("profile") {
            Ok(Some(v)) => v.into(),
            Ok(None) => ctx
                .maybe_get_env("DTU_S3_PROFILE")
                .unwrap_or_else(|| "dtu".into()),
            Err(_) => return Err(cfg.invalid_key("profile", "str")),
        };

        let bin = match cfg.maybe_get_str("aws-bin") {
            Ok(Some(v)) => v.into(),
            Ok(None) => ctx.get_bin("aws")?,
            Err(_) => return Err(cfg.invalid_key("bin", "str")),
        };

        let cache = cfg.get_bool_or("cache", true);
        let cache_is_link = cfg.get_bool_or("cache-is-link", false);
        Ok(Self {
            bin,
            bucket,
            profile,
            cache,
            cache_is_link,
        })
    }

    fn to_s3_url(&self, path: &str) -> String {
        format!("s3://{}/{}", self.bucket, path.trim_start_matches('/'))
    }

    fn rm_s3(&self, path: &str) -> crate::Result<()> {
        let url = self.to_s3_url(path);
        let args = &["--profile", self.profile.as_str(), "s3", "rm", url.as_str()];
        if let Err(crate::Error::CommandError(stat, output)) =
            run_cmd(&self.bin, args)?.err_on_status()
        {
            return Err(
                crate::Error::CommandError(stat, format!("aws rm failed: {}", output)).into(),
            );
        }
        Ok(())
    }

    fn list_s3(&self, prefix: Option<&str>) -> crate::Result<Vec<String>> {
        let pre = prefix.unwrap_or("");
        let args = &[
            "--output",
            "json",
            "--profile",
            self.profile.as_str(),
            "s3api",
            "list-objects-v2",
            "--bucket",
            self.bucket.as_str(),
            "--prefix",
            pre,
        ];
        let output = match run_cmd(&self.bin, args)?.err_on_status() {
            Err(crate::Error::CommandError(stat, output)) => {
                return Err(crate::Error::CommandError(
                    stat,
                    format!("aws s3api list-objects-v2 failed: {}", output),
                )
                .into())
            }
            Err(e) => return Err(e.into()),
            Ok(v) => v,
        };
        let sout = output.stdout_utf8_lossy();
        let trimmed = sout.trim();
        if trimmed.len() == 0 {
            return Ok(Vec::new());
        }
        Ok(serde_json::from_str::<AwsListResult>(trimmed)
            .map_err(|e| {
                crate::Error::Generic(format!("invalid result from aws list command: {e}"))
            })?
            .into())
    }

    fn s3_cp(&self, src: &str, dst: &str) -> crate::Result<()> {
        let args = &["--profile", self.profile.as_ref(), "s3", "cp", src, dst];
        let res = run_cmd(&self.bin, args);

        if let Err(crate::Error::CommandError(stat, output)) = res?.err_on_status() {
            return Err(crate::Error::CommandError(
                stat,
                format!("aws cp failed: {}", output),
            ));
        }
        Ok(())
    }

    fn s3_get(&self, ctx: &dyn Context, remote: &str, local: &str) -> crate::Result<()> {
        let s3_url = self.to_s3_url(remote);
        self.s3_cp(&s3_url, local)?;
        if !self.cache {
            return Ok(());
        }

        if let Err(e) = self.create_cache_file(ctx, remote, local) {
            log::error!("failed to create cache file for {}: {}", remote, e);
        }

        Ok(())
    }

    fn create_cache_file(&self, ctx: &dyn Context, remote: &str, local: &str) -> crate::Result<()> {
        let cache_dir = self.get_cache_dir(ctx, true)?;
        let filename = Self::remote_to_cache(remote);
        let cache_path = cache_dir.join(filename.as_ref());
        fs::copy(local, &cache_path)?;
        Ok(())
    }

    fn get_from_cache(&self, ctx: &dyn Context, remote: &str, local: &str) -> crate::Result<bool> {
        let cache_dir = self.get_cache_dir(ctx, true)?;
        let filename = Self::remote_to_cache(remote);
        let cache_path = cache_dir.join(filename.as_ref());
        if !cache_path.exists() {
            return Ok(false);
        }
        if self.cache_is_link {
            maybe_link(&cache_path, local)?;
        } else {
            fs::copy(&cache_path, local)?;
        }
        Ok(true)
    }

    fn get_cache_dir(&self, ctx: &dyn Context, create: bool) -> crate::Result<PathBuf> {
        let cache_dir = ctx.get_cache_dir()?.join("s3");
        if create && !cache_dir.exists() {
            ensure_dir_exists(&cache_dir)?;
        }
        Ok(cache_dir)
    }

    fn remote_to_cache<'a>(remote: &'a str) -> Cow<'a, str> {
        if !remote.contains(OS_PATH_SEP_CHAR) {
            Cow::Borrowed(remote)
        } else {
            Cow::Owned(replace_char(remote, OS_PATH_SEP_CHAR, SQUASH_PATH_SEP_CHAR))
        }
    }
}

impl FileStore for S3FileStore {
    fn name(&self) -> &'static str {
        S3FileStore::NAME
    }

    fn list_files(&self, _ctx: &dyn Context, dir: Option<&str>) -> crate::Result<Vec<String>> {
        self.list_s3(dir)
    }

    fn remove_file(&self, _ctx: &dyn Context, file: &str) -> crate::Result<()> {
        self.rm_s3(file)
    }

    fn put_file(
        &self,
        _ctx: &dyn Context,
        local_path: &str,
        remote_path: &str,
    ) -> crate::Result<()> {
        let s3_url = self.to_s3_url(remote_path);
        self.s3_cp(local_path, &s3_url)
    }

    fn get_file(
        &self,
        ctx: &dyn Context,
        remote_path: &str,
        local_path: &str,
    ) -> crate::Result<()> {
        if !self.cache {
            return self.s3_get(ctx, remote_path, local_path);
        }

        let cache_dir = ctx.get_cache_dir()?.join("s3");
        if !cache_dir.exists() {
            return self.s3_get(ctx, remote_path, local_path);
        }
        if matches!(self.get_from_cache(ctx, remote_path, local_path), Ok(true)) {
            return Ok(());
        }
        self.s3_get(ctx, remote_path, local_path)
    }
}

#[cfg(test)]
mod test {

    use crate::{
        testing::{tmp_context, TestContext},
        utils::ensure_dir_exists,
    };

    use super::*;
    use rstest::*;

    fn write_config(ctx: &TestContext, content: &str) -> PathBuf {
        let cfg = ctx.get_user_config_dir().unwrap();
        ensure_dir_exists(&cfg).unwrap();
        let file = cfg.join("config.toml");
        fs::write(&file, content).expect("writing config toml file");
        file
    }

    #[rstest]
    fn test_default_fallback(tmp_context: TestContext) {
        write_config(&tmp_context, "");
        let store = get_filestore(&tmp_context).expect("should have gotten a default filestore");
        assert_eq!(
            store.name(),
            LocalFileStore::NAME,
            "should have defaulted to local"
        );
    }

    #[rstest]
    fn test_s3_config(tmp_context: TestContext) {
        write_config(
            &tmp_context,
            r#"[filestore.s3]
bucket = "neato"
aws-bin = "aws"
"#,
        );
        let store = get_filestore(&tmp_context).expect("should have gotten a file store");
        assert_eq!(
            store.name(),
            S3FileStore::NAME,
            "should have gotten an S3 store"
        );
    }

    #[rstest]
    fn test_local_config(tmp_context: TestContext) {
        write_config(
            &tmp_context,
            r#"[filestore.local]
base = "such/path"
"#,
        );
        let store = get_filestore(&tmp_context).expect("should have gotten a file store");
        assert_eq!(
            store.name(),
            LocalFileStore::NAME,
            "should have gotten a local store"
        );
    }

    #[rstest]
    fn test_env_fallback(mut tmp_context: TestContext) {
        tmp_context.set_env("DTU_S3_BUCKET", "envbucket");
        tmp_context.set_env("DTU_S3_AWS_BIN", "aws");
        let store = get_filestore(&tmp_context).expect("should have gotten a file store");
        assert_eq!(
            store.name(),
            S3FileStore::NAME,
            "should have gotten the S3 store"
        );
    }
}
