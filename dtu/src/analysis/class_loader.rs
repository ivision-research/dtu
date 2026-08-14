use std::fs::File;
use std::sync::Arc;

use anyhow::{bail, Context as AnyhowContext};
use smalisa::ssa::SsaClass;
use smalisa::{parse_class, Arena, Lexer, Parser};

use crate::analysis::yokecache::{self, YokeCache, Yoked};
use crate::utils::ClassName;
use crate::{
    db::graph::{models::ClassId, FRAMEWORK_SOURCE},
    utils::{find_smali_file_for_class, DevicePath},
    Context,
};

pub struct SsaClassLoader(YokeCache<ClassId, SsaClass<'static>>);

impl SsaClassLoader {
    pub fn new(ctx: &dyn Context) -> Option<Self> {
        YokeCache::new(ctx, "ssa_class_cache.db").map(Self)
    }

    pub fn get_ssa_class(
        &self,
        ctx: &dyn Context,
        class_id: ClassId,
        class_name: &ClassName,
        source: &String,
    ) -> anyhow::Result<Arc<Yoked<SsaClass<'static>>>> {
        self.0
            .get(class_id, || build_ssa_form(ctx, class_name, source))
    }
}

fn build_ssa_form(
    ctx: &dyn Context,
    class_name: &ClassName,
    source: &String,
) -> anyhow::Result<Vec<u8>> {
    // TODO: Brittle? This should be solved somewhere else (maybe already, I should hunt that down)
    let apk = if source == FRAMEWORK_SOURCE {
        None
    } else {
        Some(DevicePath::from_squashed(source))
    };
    let Some(sf) = find_smali_file_for_class(ctx, &class_name, apk.as_ref()) else {
        bail!("failed to find smali file for {} in {}", class_name, source);
    };

    let file = File::open(&sf).with_context(|| format!("opening file {}", sf.display()))?;

    let arena = Arena::new();
    let lexer = Lexer::new_buffered(file, &arena);
    let mut parser = Parser::new(lexer);
    let Ok(class) = parse_class(&mut parser) else {
        bail!(
            "failed to parse class {} in file {}",
            class_name,
            sf.display()
        );
    };
    let class = SsaClass::from_class(class)?;
    yokecache::serialize(&class)
}
