use mockall::mock;
use rstest::fixture;

use dtu;
use dtu::config::Config;

#[fixture]
pub fn mock_context() -> MockContext {
    MockContext::new()
}

mock! {
    pub Context {

    }

    impl dtu::Context for Context {
        fn get_target_api_level(&self) -> u32;
        fn maybe_get_env(&self, key: &str) -> Option<String>;
        fn maybe_get_bin(&self, bin: &str) -> Option<String>;
        fn get_project_config<'a>(&'a self) -> dtu::Result<Option<&'a Config>>;
    }
}
