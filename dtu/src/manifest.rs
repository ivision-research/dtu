use quick_xml::events::{attributes::Attribute, BytesStart, Event};
use serde::Deserialize;
use std::{
    borrow::Cow,
    collections::HashSet,
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
};

use crate::utils::{open_file, path_must_str};

pub trait ManifestResolver {
    /// Convert a manifest value into a string, resolving all references
    ///
    /// If the given value is not a reference, returns the value itself
    fn resolve_string<'v>(&self, value: &'v str) -> Cow<'v, str>;

    /// Convert a manifest value into a boolean, resolving all references
    ///
    /// If the given value is not a reference, it is directly converted to a boolean
    fn resolve_bool(&self, value: &str) -> Option<bool>;

    /// Resolve a string array reference
    fn resolve_string_array(&self, reference: &str) -> Option<Vec<String>>;

    /// Get a string by the ID (0x...) present in smali code
    fn resolve_string_id(&self, id: &str) -> Option<String>;
}

impl<T: ManifestResolver> ManifestResolver for Option<T> {
    fn resolve_bool(&self, value: &str) -> Option<bool> {
        match self {
            Some(v) => v.resolve_bool(value),
            None => get_bool_resource(value),
        }
    }
    fn resolve_string<'v>(&self, value: &'v str) -> Cow<'v, str> {
        match self {
            Some(v) => v.resolve_string(value),
            None => Cow::Borrowed(value),
        }
    }

    fn resolve_string_id(&self, id: &str) -> Option<String> {
        match self {
            Some(v) => v.resolve_string_id(id),
            None => None,
        }
    }

    fn resolve_string_array(&self, reference: &str) -> Option<Vec<String>> {
        match self {
            Some(v) => v.resolve_string_array(reference),
            None => None,
        }
    }
}

/// Implementation of ManifestResolver that doesn't do anything
pub struct NoopManifestResolver {}

impl Default for NoopManifestResolver {
    fn default() -> Self {
        Self {}
    }
}

impl ManifestResolver for NoopManifestResolver {
    fn resolve_bool(&self, value: &str) -> Option<bool> {
        get_bool_resource(value)
    }

    fn resolve_string<'v>(&self, value: &'v str) -> Cow<'v, str> {
        Cow::Borrowed(value)
    }

    fn resolve_string_id(&self, _id: &str) -> Option<String> {
        None
    }

    fn resolve_string_array(&self, _reference: &str) -> Option<Vec<String>> {
        None
    }
}

/// A ManifestResolver implementation that uses apktool output to resolve references
pub struct ApktoolManifestResolver {
    base_dir: PathBuf,
}

type XmlReader = quick_xml::Reader<BufReader<File>>;

fn get_bool_resource(raw: &str) -> Option<bool> {
    if raw == "true" {
        Some(true)
    } else if raw == "false" {
        Some(false)
    } else {
        None
    }
}

/// Holds the kind and name of a reference
#[cfg_attr(test, derive(Debug, PartialEq))]
struct ValueReference<'a> {
    kind: &'a str,
    name: &'a str,
}

impl<'a> ValueReference<'a> {
    fn parse(from: &'a str) -> Option<Self> {
        let (kind, name) = &from[1..].split_once('/')?;
        Some(Self { kind, name })
    }

    fn tag_for(&self) -> &'a str {
        // TODO check this
        self.kind
    }
}

type Transformer<T> = fn(&str) -> Option<T>;

struct XmlResolver<'x, T> {
    resolver: &'x ApktoolManifestResolver,
    xml: XmlReader,
    tag: &'x str,
    key: &'x str,
    seen: &'x mut HashSet<String>,
    transform: &'x Transformer<T>,
}

fn get_attribute_value(bs: &BytesStart, name: &str) -> Option<String> {
    for e in bs.attributes() {
        let att = match e {
            Ok(v) => v,
            Err(_) => continue,
        };

        if String::from_utf8_lossy(att.key.local_name().as_ref()) == name {
            return match String::from_utf8_lossy(&att.value) {
                Cow::Owned(s) => Some(s),
                Cow::Borrowed(s) => Some(String::from(s)),
            };
        }
    }
    None
}

fn has_attribute_value(bs: &BytesStart, name: &str, expected: &str) -> bool {
    for e in bs.attributes() {
        let att = match e {
            Ok(v) => v,
            Err(_) => continue,
        };

        if String::from_utf8_lossy(att.key.local_name().as_ref()) != name {
            continue;
        }

        let attval = String::from_utf8_lossy(&att.value);
        return attval == expected;
    }
    false
}

impl<'x, T> XmlResolver<'x, T> {
    fn for_value_reference(
        resolver: &'x ApktoolManifestResolver,
        vr: &'x ValueReference,
        transform: &'x Transformer<T>,
        seen: &'x mut HashSet<String>,
    ) -> Option<Self> {
        let path = resolver.get_xml_path_for(vr);
        let file = match open_file(&path) {
            Err(e) => {
                log::error!("failed to read path {}: {}", path_must_str(&path), e);
                return None;
            }
            Ok(v) => v,
        };
        let xml = quick_xml::Reader::from_reader(BufReader::new(file));
        Some(Self {
            xml,
            tag: vr.tag_for(),
            resolver,
            seen,
            transform,
            key: vr.name,
        })
    }

    fn check_name(&self, att: &Attribute) -> bool {
        let value = String::from_utf8_lossy(&att.value);
        value.as_ref() == self.key
    }

    /// Advance the XML parser to the <$self.tag name="$self.name"> tag
    fn goto_value_entry(&mut self, buf: &mut Vec<u8>) -> bool {
        let tag_bytes = self.tag.as_bytes();

        loop {
            // Loop through the given XML and look for <$self.tag name="$self.name">
            match self.xml.read_event_into(buf) {
                Ok(Event::Eof) => break,
                Ok(Event::Start(bs)) if bs.local_name().as_ref() == tag_bytes => {
                    let attributes = bs.attributes();
                    for e in attributes {
                        let att = match e {
                            Ok(v) => v,
                            Err(_) => continue,
                        };

                        if att.key.local_name().as_ref() != b"name" {
                            continue;
                        }

                        // Check if the name is the name we were given
                        if !self.check_name(&att) {
                            break;
                        }

                        return true;
                    }
                }
                Err(e) => {
                    log::error!("error reading XML event: {}", e);
                    break;
                }
                _ => continue,
            }

            buf.clear();
        }
        false
    }

    fn expect_next_tag(&mut self, buf: &mut Vec<u8>, name: &str) -> Option<bool> {
        match self.xml.read_event_into(buf) {
            Ok(Event::Start(bs)) => Some(bs.local_name().as_ref() == name.as_bytes()),
            Err(e) => {
                log::error!("error reading XML event: {}", e);
                return None;
            }
            _ => None,
        }
    }

    fn expect_next_text<'v>(&mut self, buf: &'v mut Vec<u8>) -> Option<Cow<'v, str>> {
        match self.xml.read_event_into(buf) {
            Ok(Event::Text(bt)) => match bt.unescape() {
                Ok(txt) => Some(txt),
                // TODO: Hm....
                Err(_) => {
                    let inner_bytes = bt.into_inner();
                    let as_utf8 = String::from_utf8_lossy(&inner_bytes);
                    log::warn!("invalid escape for text: {}", as_utf8);
                    None
                }
            },
            // <foo></foo> ?
            Ok(Event::End(_)) => None,
            _ => None,
        }
    }

    fn resolve_array(&mut self) -> Option<Vec<T>> {
        let mut result: Vec<T> = Vec::new();
        let mut buf = Vec::new();

        if !self.goto_value_entry(&mut buf) {
            return None;
        }
        buf.clear();

        // Now there should be a bunch of `<item>ITEM</item>` elements

        loop {
            match self.expect_next_tag(&mut buf, "item") {
                Some(false) | None => break,
                _ => {}
            }

            buf.clear();
            let txt = match self.expect_next_text(&mut buf) {
                Some(v) => v,
                None => continue,
            };

            if !txt.starts_with("@") {
                match (self.transform)(&txt) {
                    Some(v) => result.push(v),
                    None => {
                        log::warn!("failed to transform {}", txt);
                        continue;
                    }
                }
            }

            let vr = match self.recursive_reference_target(&txt) {
                Some(v) => v,
                None => {
                    log::warn!("skipping invalid array reference item: {}", txt);
                    continue;
                }
            };

            let mut resolver = match XmlResolver::for_value_reference(
                self.resolver,
                &vr,
                self.transform,
                self.seen,
            ) {
                Some(v) => v,
                None => {
                    log::warn!("failed to parse reference inside of array: {}", txt);
                    continue;
                }
            };

            match resolver.resolve() {
                None => {
                    log::warn!("failed to resolve reference inside of array: {}", txt);
                    continue;
                }
                Some(v) => result.push(v),
            }
        }

        Some(result)
    }

    fn recursive_reference_target<'r>(&mut self, reference: &'r str) -> Option<ValueReference<'r>> {
        if self.seen.contains(reference) {
            return None;
        }
        self.seen.insert(String::from(reference));

        ValueReference::parse(reference)
    }

    fn resolve(&mut self) -> Option<T> {
        let mut buf = Vec::new();

        if !self.goto_value_entry(&mut buf) {
            return None;
        }
        buf.clear();

        let txt = match self.xml.read_event_into(&mut buf) {
            Ok(Event::Text(bt)) => match bt.unescape() {
                Ok(txt) => txt,
                Err(_) => {
                    let inner_bytes = bt.into_inner();
                    let as_utf8 = String::from_utf8_lossy(&inner_bytes);
                    log::warn!("invalid escape for text: {}", as_utf8);
                    return None;
                }
            },
            _ => return None,
        };

        if !txt.starts_with("@") {
            return (self.transform)(&txt);
        }

        let vr = match self.recursive_reference_target(&txt) {
            Some(v) => v,
            // Give a chance to surface the value I guess, helpful if it is a string to give
            // something instead of nothing
            None => return (self.transform)(&txt),
        };

        let mut resolver =
            match XmlResolver::for_value_reference(self.resolver, &vr, self.transform, self.seen) {
                Some(v) => v,
                None => return (self.transform)(&txt),
            };

        resolver.resolve()
    }
}

impl ManifestResolver for ApktoolManifestResolver {
    fn resolve_bool(&self, value: &str) -> Option<bool> {
        match self.resolve(value, get_bool_resource) {
            None => get_bool_resource(value),
            Some(v) => Some(v),
        }
    }

    fn resolve_string<'v>(&self, value: &'v str) -> Cow<'v, str> {
        match self.resolve(value, |s| Some(String::from(s))) {
            Some(v) => Cow::Owned(v),
            None => Cow::Borrowed(value),
        }
    }

    fn resolve_string_array(&self, reference: &str) -> Option<Vec<String>> {
        self.resolve_array(reference, |s| Some(String::from(s)))
    }

    fn resolve_string_id(&self, id: &str) -> Option<String> {
        let name = self.get_public_xml_name(id)?;
        match self.resolve_string(&name) {
            Cow::Owned(s) => Some(s),
            Cow::Borrowed(s) => Some(String::from(s)),
        }
    }
}

impl ApktoolManifestResolver {
    /// Create a new ManifestResolver using an apktool output directory
    pub fn new(base_dir: &Path) -> Self {
        Self::new_from_pathbuf(PathBuf::from(base_dir))
    }

    pub fn new_from_pathbuf(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    /// Generic resolve function for @KIND/KEY values in manifests
    ///
    /// Note that this function will return None if the passed value isn't in the @KIND/KEY format
    fn resolve<T>(&self, value: &str, transform: Transformer<T>) -> Option<T> {
        if !value.starts_with("@") {
            return None;
        }

        let mut seen: HashSet<String> = HashSet::new();
        let reference = ValueReference::parse(value)?;

        let mut resolver =
            XmlResolver::for_value_reference(self, &reference, &transform, &mut seen)?;
        resolver.resolve()
    }

    fn resolve_array<T>(&self, reference: &str, transform: Transformer<T>) -> Option<Vec<T>> {
        if !reference.starts_with("@") {
            return None;
        }

        let mut seen: HashSet<String> = HashSet::new();
        let vr = ValueReference::parse(reference)?;

        let mut resolver = XmlResolver::for_value_reference(self, &vr, &transform, &mut seen)?;
        resolver.resolve_array()
    }

    fn get_xml_path_for(&self, vr: &ValueReference) -> PathBuf {
        // TODO: string list
        match vr.kind {
            _ => self.base_dir.join(format!("res/values/{}s.xml", vr.kind)),
        }
    }

    /// Retrieve the `name` field for the given ID in the `public.xml` file
    fn get_public_xml_name(&self, id: &str) -> Option<String> {
        if !id.starts_with("0x") {
            return None;
        }
        let path = self.base_dir.join("values/public.xml");
        let file = match open_file(&path) {
            Err(e) => {
                log::error!("failed to read path {}: {}", path_must_str(&path), e);
                return None;
            }
            Ok(v) => v,
        };
        let mut xml = quick_xml::Reader::from_reader(BufReader::new(file));
        let mut buf = Vec::new();

        loop {
            let bs = match xml.read_event_into(&mut buf) {
                Ok(Event::Start(bs)) if bs.local_name().as_ref() == b"public" => bs,
                Ok(Event::Eof) => break,
                Err(e) => {
                    log::error!("error parsing XML file {}: {}", path_must_str(&path), e);
                    return None;
                }
                _ => continue,
            };

            if has_attribute_value(&bs, "id", id) {
                return get_attribute_value(&bs, "name");
            }
        }
        None
    }
}

macro_rules! cow_getter {
    ($field:ident) => {
        pub fn $field<'s>(&'s self, resolver: &dyn ManifestResolver) -> Cow<'s, str> {
            resolver.resolve_string(&self.$field)
        }
    };
}

macro_rules! maybe_cow_getter {
    ($field:ident) => {
        pub fn $field<'s>(&'s self, resolver: &dyn ManifestResolver) -> Option<Cow<'s, str>> {
            match &self.$field {
                None => None,
                Some(v) => Some(resolver.resolve_string(v)),
            }
        }
    };
}

macro_rules! named {
    ($type:ident) => {
        impl $type {
            cow_getter!(name);
        }
    };
}

#[derive(Deserialize)]
pub struct Action {
    #[serde(rename = "@name")]
    name: String,
}

named!(Action);

#[derive(Deserialize)]
pub struct Category {
    #[serde(rename = "@name")]
    name: String,
}
named!(Category);

#[derive(Deserialize)]
pub struct Data {
    #[serde(rename = "@scheme")]
    scheme: Option<String>,
    #[serde(rename = "@host")]
    host: Option<String>,
    #[serde(rename = "@port")]
    port: Option<String>,
    #[serde(rename = "@path")]
    path: Option<String>,
    #[serde(rename = "@pathPattern")]
    path_pattern: Option<String>,
    #[serde(rename = "@pathPrefix")]
    path_prefix: Option<String>,
    #[serde(rename = "@pathSuffix")]
    path_suffix: Option<String>,
    #[serde(rename = "@pathAdvancedPattern")]
    path_advanced_pattern: Option<String>,
    #[serde(rename = "@mimeType")]
    mime_type: Option<String>,
}

impl Data {
    maybe_cow_getter!(scheme);
    maybe_cow_getter!(host);
    maybe_cow_getter!(port);
    maybe_cow_getter!(path);
    maybe_cow_getter!(path_pattern);
    maybe_cow_getter!(path_prefix);
    maybe_cow_getter!(path_suffix);
    maybe_cow_getter!(path_advanced_pattern);
    maybe_cow_getter!(mime_type);
}

#[derive(Deserialize)]
pub struct IntentFilter {
    #[serde(rename = "action", default = "Vec::new")]
    actions: Vec<Action>,

    #[serde(rename = "category", default = "Vec::new")]
    categories: Vec<Category>,

    #[serde(default = "Vec::new")]
    data: Vec<Data>,
}

impl IntentFilter {
    pub fn get_actions(&self) -> &[Action] {
        self.actions.as_slice()
    }

    pub fn get_categories(&self) -> &[Category] {
        self.categories.as_slice()
    }
    pub fn get_data(&self) -> &[Data] {
        self.data.as_slice()
    }
}

pub trait IPC {
    /// Retrieve the [ClassName] of the item
    ///
    /// Items are often given short names like `.ClassName` that are relative to the Manifest
    /// package. This type does not have that information, so the returned [ClassName] may need to
    /// be updated with the package if that is the case
    fn name<'s>(&'s self, resolver: &dyn ManifestResolver) -> Cow<'s, str>;

    /// Retrive the exported attribute from the item
    ///
    /// This action may fail if the exported attribute is not a valid boolean string ("true" or
    /// "false") or cannot be resolved. Note that None will not be returned in the case that the
    /// exported attribute is missing because this attribute defaults to `false`.
    ///
    /// Note that this takes into account the presence of <intent-filter> when determining the
    /// default value
    fn exported(&self, resolver: &dyn ManifestResolver) -> Option<bool>;

    /// Retrieve the enabled attribute from the item
    ///
    /// See [exported] for why this returns an `Option`
    fn enabled(&self, resolver: &dyn ManifestResolver) -> Option<bool>;

    /// Retrieve the required permission if one exists, resolving references
    fn permission<'s>(&'s self, resolver: &dyn ManifestResolver) -> Option<Cow<'s, str>>;
}

macro_rules! def_ipc {
    ($name:ident) => {
        def_ipc!($name {});
    };
    ($name:ident { $($rem:tt)* }) => {
        #[derive(Deserialize)]
        pub struct $name {
            #[serde(rename = "@name")]
            name: String,

            #[serde(rename = "@enabled")]
            enabled: Option<String>,

            #[serde(rename = "@exported")]
            exported: Option<String>,

            #[serde(rename = "@permission")]
            permission: Option<String>,

            #[serde(rename = "intent-filter", default = "Vec::new")]
            pub intent_filters: Vec<IntentFilter>,

            $($rem)*
        }

        impl IPC for $name {
            fn name<'s>(&'s self, resolver: &dyn ManifestResolver) -> Cow<'s, str> {
                resolver.resolve_string(&self.name)
            }

            fn exported(&self, resolver: &dyn ManifestResolver) -> Option<bool> {
                match self.exported {
                    None => Some(self.intent_filters.len() > 0),
                    Some(ref ex) => resolver.resolve_bool(&ex),
                }
            }

            fn enabled(&self, resolver: &dyn ManifestResolver) -> Option<bool> {
                match self.enabled {
                    None => Some(true),
                    Some(ref en) => resolver.resolve_bool(en),
                }
            }

            fn permission<'s>(&'s self, resolver: &dyn ManifestResolver) -> Option<Cow<'s, str>> {
                let perm = self.permission.as_ref()?;
                Some(resolver.resolve_string(perm))
            }
        }
    };
}

def_ipc!(Activity);
def_ipc!(Receiver);
def_ipc!(Service);
def_ipc!(Provider {
    #[serde(rename = "@authorities")]
    authorities: String,

    #[serde(rename = "@readPermission")]
    read_permission: Option<String>,

    #[serde(rename = "@writePermission")]
    write_permission: Option<String>,

    #[serde(rename = "grantUriPermissions")]
    grant_uri_permissions: Option<String>,
});

impl Provider {
    pub fn authorities<'s>(&'s self, resolver: &dyn ManifestResolver) -> Cow<'s, str> {
        // We don't normally look for @, but this will prevent making a new string when ; is
        // present
        if !self.authorities.contains('@') {
            return Cow::Borrowed(self.authorities.as_str());
        }

        if !self.authorities.contains(';') {
            return resolver.resolve_string(&self.authorities);
        }

        let total = self.authorities.chars().filter(|it| *it == ';').count() - 1;

        let mut s = String::with_capacity(self.authorities.len());
        let split = self.authorities.split(';');
        for (i, auth) in split.enumerate() {
            s.push_str(&resolver.resolve_string(&auth));
            if i <= total {
                s.push(';');
            }
        }

        Cow::Owned(s)
    }

    pub fn grant_uri_permissions(&self, resolver: &dyn ManifestResolver) -> Option<bool> {
        match &self.grant_uri_permissions {
            None => Some(false),
            Some(v) => resolver.resolve_bool(v),
        }
    }

    pub fn read_permission<'s>(&'s self, resolver: &dyn ManifestResolver) -> Option<Cow<'s, str>> {
        match self.read_permission {
            None => self.permission(resolver),
            Some(ref v) => Some(resolver.resolve_string(v)),
        }
    }

    pub fn write_permission<'s>(&'s self, resolver: &dyn ManifestResolver) -> Option<Cow<'s, str>> {
        match self.write_permission {
            None => self.permission(resolver),
            Some(ref v) => Some(resolver.resolve_string(v)),
        }
    }
}

#[derive(Deserialize)]
pub struct Application {
    #[serde(rename = "@debuggable")]
    debuggable: Option<String>,

    #[serde(rename = "@allowBackup")]
    allow_backup: Option<String>,

    #[serde(rename = "activity", default = "Vec::new")]
    pub activities: Vec<Activity>,

    #[serde(rename = "activity-alias", default = "Vec::new")]
    pub activity_aliases: Vec<Activity>,

    #[serde(rename = "provider", default = "Vec::new")]
    pub providers: Vec<Provider>,

    #[serde(rename = "receiver", default = "Vec::new")]
    pub receivers: Vec<Receiver>,

    #[serde(rename = "service", default = "Vec::new")]
    pub services: Vec<Service>,
}

#[cfg_attr(test, derive(Debug, PartialEq))]
#[derive(Deserialize)]
pub struct UsesPermission {
    #[serde(rename = "@name")]
    name: String,
}

named!(UsesPermission);

impl<S> PartialEq<S> for UsesPermission
where
    S: AsRef<str>,
{
    fn eq(&self, other: &S) -> bool {
        self.name == other.as_ref()
    }
}

#[cfg_attr(test, derive(Debug, PartialEq))]
#[derive(Deserialize)]
pub struct Permission {
    #[serde(rename = "@name")]
    name: String,

    #[serde(rename = "@protectionLevel")]
    protection_level: Option<String>,
}

impl Permission {
    cow_getter!(name);

    pub fn protection_level<'s>(&'s self, resolver: &dyn ManifestResolver) -> Cow<'s, str> {
        match &self.protection_level {
            Some(v) => resolver.resolve_string(v),
            None => Cow::Borrowed("normal"),
        }
    }
}

impl<S> PartialEq<S> for Permission
where
    S: AsRef<str>,
{
    fn eq(&self, other: &S) -> bool {
        self.name == other.as_ref()
    }
}

/// An incomplete but sufficient for our purposes Android Manifest type
///
/// This manifest type stores the raw strings found in the Manifest file, but most of these strings
/// can be -- and occasionally are -- set to references (@type/name). To deal with that, we
/// delegate most field retrieval operations to methods that take a [ManifestResolver] object and
/// attempt to resolve references. Since this operation may fail, most of these methods return
/// [Option] types.
#[derive(Deserialize)]
pub struct Manifest {
    #[serde(rename = "@package")]
    package: String,
    #[serde(rename = "uses-permission", default = "Vec::new")]
    pub uses_permissions: Vec<UsesPermission>,
    #[serde(rename = "permission", default = "Vec::new")]
    pub permissions: Vec<Permission>,
    pub application: Application,
}

impl Manifest {
    /// Parse an AndroidManifest.xml file
    ///
    /// Return None on failure, not like we're gonna try to fix it or anything.
    pub fn from_file(path: &Path) -> anyhow::Result<Self> {
        let file = match open_file(path) {
            Ok(v) => v,
            Err(e) => {
                log::error!("failed to open {}: {}", path_must_str(path), e);
                return Err(e.into());
            }
        };
        let mut br = BufReader::new(file);
        let manifest: Self = match quick_xml::de::from_reader(&mut br) {
            Ok(v) => v,
            Err(e) => {
                log::error!("failed to deserialize {}: {}", path_must_str(path), e);
                return Err(e.into());
            }
        };
        Ok(manifest)
    }

    pub fn allow_backup(&self, resolver: &dyn ManifestResolver) -> Option<bool> {
        match &self.application.allow_backup {
            None => Some(false),
            Some(v) => resolver.resolve_bool(v),
        }
    }

    pub fn get_permissions(&self) -> &[Permission] {
        self.permissions.as_slice()
    }

    pub fn get_uses_permissions(&self) -> &[UsesPermission] {
        self.uses_permissions.as_slice()
    }

    pub fn get_activities(&self) -> &[Activity] {
        self.application.activities.as_slice()
    }

    pub fn get_activity_aliases(&self) -> &[Activity] {
        self.application.activity_aliases.as_slice()
    }

    pub fn get_receivers(&self) -> &[Receiver] {
        self.application.receivers.as_slice()
    }

    pub fn get_services(&self) -> &[Service] {
        self.application.services.as_slice()
    }

    pub fn get_providers(&self) -> &[Provider] {
        self.application.providers.as_slice()
    }

    pub fn package<'s>(&'s self, resolver: &dyn ManifestResolver) -> Cow<'s, str> {
        resolver.resolve_string(&self.package)
    }

    pub fn debuggable(&self, resolver: &dyn ManifestResolver) -> Option<bool> {
        match &self.application.debuggable {
            None => Some(false),
            Some(v) => resolver.resolve_bool(v),
        }
    }
}

#[cfg(test)]
mod test {

    use crate::testing::{tmp_dir, TmpDir};
    use rstest::*;

    use super::*;

    fn parse(s: &str) -> Manifest {
        match quick_xml::de::from_str(s) {
            Ok(v) => v,
            Err(e) => panic!("failed to parse raw manifest {}:\n{}", s, e),
        }
    }

    #[test]
    fn test_simple_manifest() {
        let as_str = r#"<?xml version="1.0" encoding="utf-8"?>
<manifest xmlns:android="http://schemas.android.com/apk/res/android"
    xmlns:tools="http://schemas.android.com/tools"
    package="t.s.t">

    <uses-permission android:name="android.permission.INTERNET" />
    <uses-permission android:name="android.permission.ACCESS_COARSE_LOCATION" />

    <permission android:name="t.s.t.PERMISSIONA" />
    <permission
        android:name="t.s.t.PERMISSIONB"
        android:protectionLevel="signature" />

    <application
        android:allowBackup="true"
        android:dataExtractionRules="@xml/data_extraction_rules"
        android:fullBackupContent="@xml/backup_rules"
        android:icon="@mipmap/ic_launcher"
        android:label="@string/app_name"
        android:roundIcon="@mipmap/ic_launcher_round"
        android:supportsRtl="true"
        android:theme="@style/Theme.MyApplication"
        tools:targetApi="31">
        <activity
            android:name="t.s.t.MainActivity"
            android:exported="false" />

        <service
            android:name="t.s.t.MyService2"
            android:enabled="true"
            android:exported="true" />

        <activity
            android:name="t.s.t.OtherMainActivity"
            android:exported="true" />

        <receiver
            android:name=".MyReceiver2"
            android:enabled="true">
            <intent-filter>
                <action android:name="t.s.t.RECEIVER2_ACTION" />
                <category android:name="t.s.t.RECEIVER2_CATEGORY" />
                <data
                    android:host="t.s.t"
                    android:scheme="neato" />
            </intent-filter>
        </receiver>

        <service
            android:name="t.s.t.MyService"
            android:enabled="true"
            android:exported="true" />


        <receiver
            android:name=".MyReceiver"
            android:enabled="true"
            android:exported="true" />

        <receiver
            android:name=".MyReceiver3" />

    </application>

</manifest>
"#;

        let man = parse(as_str);
        let resolve = NoopManifestResolver::default();

        assert_eq!(man.package, "t.s.t");
        assert_eq!(man.application.allow_backup, Some("true".into()));
        assert_eq!(
            man.uses_permissions,
            vec![
                "android.permission.INTERNET",
                "android.permission.ACCESS_COARSE_LOCATION"
            ]
        );
        assert_eq!(
            man.permissions,
            vec![
                Permission {
                    name: "t.s.t.PERMISSIONA".into(),
                    protection_level: None,
                },
                Permission {
                    name: "t.s.t.PERMISSIONB".into(),
                    protection_level: Some("signature".into()),
                },
            ]
        );

        let rcvers = man.get_receivers();
        assert_eq!(rcvers.len(), 3, "expected two receivers");
        assert_eq!(
            rcvers[0].exported(&resolve),
            Some(true),
            "intent-filter should make exported default to true"
        );
        assert_eq!(
            rcvers[1].exported(&resolve),
            Some(true),
            "explicit export on receiver"
        );
        assert_eq!(
            rcvers[2].exported(&resolve),
            Some(false),
            "default value for export"
        );
    }

    macro_rules! resource_test {
        (
            $td:ident,
            $fname:literal,
            $raw:expr,
            $get:expr,
            $expected:expr
        ) => {
            let base_dir = $td.create_dir("resource_test");

            let full_name = format!("res/values/{}", $fname);

            base_dir.create_file_name(&full_name, Some($raw));

            let man = ApktoolManifestResolver::new(base_dir.get_path());

            let got = $get(&man);
            assert_eq!(got, $expected);
        };
    }

    #[rstest]
    fn test_get_string_resource(tmp_dir: TmpDir) {
        let raw = r#"<?xml version="1.0" encoding="utf-8"?>
<resources>
    <string name="neverused">neverused</string>
    <string name="stringval">string value &lt;with spaces&gt;</string>
    <string name="ref1">@string/stringval</string>
    <string name="ref2">@string/ref1</string>
</resources>
"#;
        resource_test!(
            tmp_dir,
            "strings.xml",
            raw,
            |man: &ApktoolManifestResolver| man.resolve_string("@string/stringval"),
            Cow::<'_, str>::Owned(String::from("string value <with spaces>"))
        );

        resource_test!(
            tmp_dir,
            "strings.xml",
            raw,
            |man: &ApktoolManifestResolver| man.resolve_string("not a reference"),
            Cow::<'_, str>::Borrowed("not a reference")
        );

        resource_test!(
            tmp_dir,
            "strings.xml",
            raw,
            |man: &ApktoolManifestResolver| man.resolve_string("@string/ref2"),
            Cow::<'_, str>::Owned(String::from("string value <with spaces>"))
        );
    }

    #[rstest]
    fn test_get_bool_resource(tmp_dir: TmpDir) {
        let raw = r#"<?xml version="1.0" encoding="utf-8"?>
<resources>
    <bool name="notthisone">true</bool>
    <bool name="recursive_target">true</bool>
    <bool name="thisone">false</bool>
    <bool name="recursive">@bool/recursive_target</bool>
</resources>
"#;

        resource_test!(
            tmp_dir,
            "bools.xml",
            raw,
            |man: &ApktoolManifestResolver| man.resolve_bool("@bool/thisone"),
            Some(false)
        );

        resource_test!(
            tmp_dir,
            "bools.xml",
            raw,
            |man: &ApktoolManifestResolver| man.resolve_bool("true"),
            Some(true)
        );

        resource_test!(
            tmp_dir,
            "bools.xml",
            raw,
            |man: &ApktoolManifestResolver| man.resolve_bool("@bool/recursive"),
            Some(true)
        );
    }

    #[test]
    fn test_value_reference() {
        assert_eq!(
            ValueReference::parse("@string/name"),
            Some(ValueReference {
                kind: "string",
                name: "name"
            })
        );
    }
}
