use std::fs::{self, OpenOptions};
use std::{env, path::PathBuf};

use rand::Rng;
use rstest::fixture;

pub struct TmpDir {
    temp_dir: PathBuf,
    base: bool,
}

impl TmpDir {
    pub fn get_path(&self) -> &PathBuf {
        &self.temp_dir
    }

    pub fn create_dir(&self, name: &str) -> TmpDir {
        let path = self.temp_dir.join(name);
        fs::create_dir_all(&path).expect("failed to make temp directory");
        TmpDir {
            temp_dir: path,
            base: false,
        }
    }

    pub fn create_file_name(&self, name: &str, content: Option<&str>) -> PathBuf {
        let path = self.temp_dir.join(name);

        let parent = path.parent().unwrap();

        if !parent.exists() {
            fs::create_dir_all(parent).expect("failed to create directories for new file");
        }

        match content {
            Some(content) => {
                fs::write(&path, content).expect("failed to make temp file with content")
            }
            None => {
                _ = OpenOptions::new()
                    .create(true)
                    .open(&path)
                    .expect("failed to make empty temp file")
            }
        }
        path
    }

    #[allow(dead_code)]
    pub fn create_file_suffix(&self, suffix: Option<&str>, content: Option<&str>) -> PathBuf {
        let mut rng = rand::thread_rng();
        let rand: u32 = rng.gen();
        let name = match suffix {
            None => rand.to_string(),
            Some(v) => format!("{}.{}", rand, v),
        };
        self.create_file_name(&name, content)
    }

    #[allow(dead_code)]
    pub fn create_file(&self, content: Option<&str>) -> PathBuf {
        self.create_file_suffix(None, content)
    }
}

impl Drop for TmpDir {
    fn drop(&mut self) {
        if self.base {
            _ = fs::remove_dir_all(&self.temp_dir);
        }
    }
}

#[fixture]
pub fn tmp_dir() -> TmpDir {
    let base = env::temp_dir();
    let mut rng = rand::thread_rng();
    let rand_name: u32 = rng.gen();
    let temp_dir = base.join(rand_name.to_string());
    let _ = fs::create_dir(&temp_dir);
    TmpDir {
        temp_dir,
        base: true,
    }
}
