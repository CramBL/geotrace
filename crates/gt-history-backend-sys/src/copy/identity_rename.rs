use std::path::Path;

use gt_history_types::{
    ATTR_IDENTITY, CURRENT_SCHEMA_VERSION, DatabaseRef, DbError, SCHEMA_VERSION_ATTR,
};
use hdf5::types::TypeDescriptor;
use hdf5::{Attribute, File, Group, IndexType, IterationOrder, LinkType, LocationInfo};

use super::InternalError;

pub(super) const STAGE_PREFIX: &str = "__geotrace_rename__";
// This version fits legacy integer attribute types and is reserved for recovery.
pub(crate) const RENAME_SCHEMA_VERSION: i64 = 127;
const STAGE_RECORDING: &str = "recording";
const STAGE_SOURCE: &str = "source";
const STAGE_DESTINATION: &str = "destination";
const STAGE_PREVIOUS_SCHEMA: &str = "previous_schema_version";
const ABSENT_SCHEMA_VERSION: i64 = -1;

// A new identity can require a larger string datatype.
fn overwrite_identity_attr(group: &Group, identity: &str) -> Result<(), InternalError> {
    if group.attr(ATTR_IDENTITY).is_ok() {
        group.delete_attr(ATTR_IDENTITY)?;
    }
    super::write_string_attr(group, ATTR_IDENTITY, identity)
}

/// The recording has a stage link throughout the identity update.
/// The source group remains addressable for HDF5 object references.
pub(crate) fn rename_identity(
    db_path: &Path,
    old: &str,
    new: &str,
    before_publish: impl FnOnce(&Group, &[DatabaseRef]) -> Result<(), DbError>,
) -> Result<(), InternalError> {
    if old == new {
        return Ok(());
    }
    if new.trim().is_empty() {
        log::warn!("rename_identity: not renaming {old:?} to an empty identity");
        return Ok(());
    }
    if new.contains('\0') {
        return Err(InternalError::Conflict(
            "The destination identity contains a null character".to_owned(),
        ));
    }

    let file = File::open_rw(db_path)?;
    let Some(by_id) = self::direct_root_group(&file, "by_identity")? else {
        return Ok(());
    };
    let metadata = self::direct_root_group(&file, "meta")?;
    if let Some(meta) = &metadata
        && !self::pending_stage_names(meta)?.is_empty()
    {
        return Err(DbError::StoredTrackLayoutChanged.into());
    }
    let mut sources = Vec::new();
    for (storage_group, group) in super::matching_identity_groups(&by_id, old, None) {
        if group.info()?.nlinks != 0 {
            let address = RenameIdentityPath {
                identity: old,
                storage_group: &storage_group,
            };
            sources.push(RenameSource::plan(&by_id, address, group)?);
        }
    }
    let recording_names = sources.iter().flat_map(|source| source.recordings.iter());
    if let Some(meta) = &metadata {
        self::require_unreserved_rename(meta, |pending| {
            pending.identity == old
                || (pending.identity == new
                    && recording_names
                        .clone()
                        .any(|name| name == &pending.group_name))
        })?;
    }
    if sources.is_empty() {
        log::warn!("rename_identity: identity {old:?} not found");
        return Ok(());
    }
    let destination = super::find_identity_group(&by_id, new, None)?;
    let destination_name = match &destination {
        Some((name, group)) => {
            self::require_unique_hard_group_link(&by_id, name, group)?;
            name.clone()
        }
        None => {
            let name = gt_history_types::identity_group_name(new);
            if by_id.link_exists(&name) {
                return Err(InternalError::Conflict(format!(
                    "History rename destination {name:?} requires a vacant identity path"
                )));
            }
            name
        }
    };
    let alternate_destinations = RenameIdentityPath {
        identity: new,
        storage_group: &destination_name,
    }
    .checked_alternate_groups(&by_id)?;
    for name in recording_names.clone() {
        if destination
            .as_ref()
            .is_some_and(|(_, group)| group.link_exists(name))
            || alternate_destinations
                .iter()
                .any(|group| group.link_exists(name))
        {
            return Err(InternalError::Conflict(format!(
                "cannot merge identity {old:?} into {new:?}: recording {name:?} exists in both"
            )));
        }
    }
    let previous_schema = self::schema_attribute(&file)?
        .as_ref()
        .map(self::read_integer_schema)
        .transpose()?;
    if previous_schema.is_some_and(|version| !(0..=CURRENT_SCHEMA_VERSION).contains(&version)) {
        return Err(InternalError::Conflict(
            "History rename requires a supported schema".to_owned(),
        ));
    }
    let meta = match metadata {
        Some(meta) => meta,
        None => file.create_group("meta")?,
    };
    let dst = match destination {
        Some((_, group)) => group,
        None => {
            let group = by_id.create_group(&destination_name)?;
            super::write_string_attr(&group, ATTR_IDENTITY, new)?;
            self::require_unique_hard_group_link(&by_id, &destination_name, &group)?;
            group
        }
    };

    let affected: Vec<_> = recording_names
        .flat_map(|name| {
            [old, new].map(|identity| DatabaseRef {
                identity: identity.to_owned(),
                group_name: name.clone(),
            })
        })
        .collect();
    before_publish(&by_id, &affected)?;
    let destination_path = dst.name();
    for source in sources {
        let source_path = source.group.name();
        for name in source.recordings {
            let stage_name = format!("{STAGE_PREFIX}{name}");
            let stage = meta.create_group(&stage_name)?;
            super::write_string_attr(&stage, STAGE_SOURCE, &source_path)?;
            super::write_string_attr(&stage, STAGE_DESTINATION, &destination_path)?;
            stage
                .new_attr::<i64>()
                .create(STAGE_PREVIOUS_SCHEMA)?
                .write_scalar(&previous_schema.unwrap_or(ABSENT_SCHEMA_VERSION))?;
            file.flush()?;
            let original = source.group.group(&name)?.loc_info()?;
            stage.link_hard(&format!("{source_path}/{name}"), STAGE_RECORDING)?;
            let staged = stage.group(STAGE_RECORDING)?.loc_info()?;
            if !self::same_object(&staged, &original) || staged.num_links != 2 {
                return Err(InternalError::Conflict(format!(
                    "History rename stage {stage_name:?} requires the original recording"
                )));
            }
            file.flush()?;
            self::write_schema_version(&file, Some(RENAME_SCHEMA_VERSION))?;
            self::settle_stage(&file, &meta, &stage_name, RenameResolution::Commit, || {
                Ok(())
            })?;
        }
    }

    log::info!("Renamed history identity {old:?} to {new:?}");
    Ok(())
}

pub(super) fn require_unstaged_recording(
    identity: &Group,
    recording_name: &str,
) -> Result<(), InternalError> {
    let Ok(meta) = identity.file()?.group("meta") else {
        return Ok(());
    };
    if meta.info()?.nlinks == 0 {
        return Ok(());
    }
    let found = identity.group(recording_name)?.loc_info()?;
    for name in meta
        .member_names()?
        .into_iter()
        .filter(|name| name.starts_with(STAGE_PREFIX))
    {
        let stage = meta.group(&name)?;
        if stage.link_exists(STAGE_RECORDING) {
            let staged = stage.group(STAGE_RECORDING)?.loc_info()?;
            if self::same_object(&staged, &found) {
                return Err(DbError::StoredTrackLayoutChanged.into());
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum RenameResolution {
    Commit,
    Recover,
}

struct RenameSource {
    group: Group,
    recordings: Vec<String>,
}

fn require_unreserved_rename(
    meta: &Group,
    conflicts: impl Fn(&DatabaseRef) -> bool,
) -> Result<(), InternalError> {
    for name in meta
        .member_names()?
        .into_iter()
        .filter(|name| name.starts_with(super::REPLACEMENT_STAGE_PREFIX))
    {
        let stage = meta.group(&name)?;
        if super::read_complete_replacement(&stage, &name)
            .as_ref()
            .is_some_and(&conflicts)
        {
            return Err(DbError::StoredTrackLayoutChanged.into());
        }
    }
    Ok(())
}

impl RenameSource {
    fn plan(
        by_id: &Group,
        address: RenameIdentityPath<'_>,
        group: Group,
    ) -> Result<Self, InternalError> {
        self::require_unique_hard_group_link(by_id, address.storage_group, &group)?;
        let alternates = address.checked_alternate_groups(by_id)?;
        let links = group.links(IndexType::Name, IterationOrder::Increasing)?;
        let mut recordings = Vec::with_capacity(links.len());
        let identity = address.identity;
        for (name, link) in links {
            if link.link_type != LinkType::Hard {
                return Err(InternalError::Conflict(format!(
                    "Recording {identity:?}/{name:?} requires a hard link"
                )));
            }
            let recording = group.group(&name)?;
            if recording.loc_info()?.num_links != 1 {
                return Err(InternalError::Conflict(format!(
                    "Recording {identity:?}/{name:?} has multiple hard links"
                )));
            }
            if super::read_recording_meta(&recording).is_none() {
                return Err(InternalError::Conflict(format!(
                    "Recording {identity:?}/{name:?} requires recording metadata"
                )));
            }
            if super::recording_identity(&recording).is_some_and(|stored| stored != identity) {
                return Err(InternalError::Conflict(format!(
                    "Recording {identity:?}/{name:?} differs from its stored identity"
                )));
            }
            if alternates
                .iter()
                .any(|alternate| alternate.link_exists(&name))
            {
                return Err(InternalError::Conflict(format!(
                    "ambiguous stored recording {identity:?}/{name:?}"
                )));
            }
            recordings.push(name);
        }
        Ok(Self { group, recordings })
    }
}

struct RenameEndpoint {
    group: Group,
    identity: String,
}

struct RenameIdentityPath<'a> {
    identity: &'a str,
    storage_group: &'a str,
}

impl RenameIdentityPath<'_> {
    fn checked_alternate_groups(&self, by_id: &Group) -> Result<Vec<Group>, InternalError> {
        let mut selected = false;
        let mut alternate_groups = Vec::new();
        for candidate in gt_history_types::identity_group_candidates(self.identity) {
            if candidate == self.storage_group {
                selected = true;
                continue;
            }
            if !by_id.link_exists(&candidate) {
                continue;
            }
            if self::link_type(by_id, &candidate)? != Some(LinkType::Hard) {
                return Err(InternalError::Conflict(format!(
                    "History rename alternate identity {candidate:?} requires a hard link"
                )));
            }
            let group = by_id.group(&candidate)?;
            if super::identity_for_group(&group, &candidate) == self.identity {
                alternate_groups.push(group);
            }
        }
        if !selected {
            return Err(InternalError::Conflict(format!(
                "History rename endpoint {:?} differs from its stored identity",
                self.storage_group,
            )));
        }
        Ok(alternate_groups)
    }
}

struct RenameStage<'a> {
    recording_name: &'a str,
    recording: Group,
    source: RenameEndpoint,
    destination: RenameEndpoint,
    schema: Option<i64>,
    previous_schema: Option<i64>,
    previous_identity: Option<String>,
}

fn pending_stage_names(meta: &Group) -> Result<Vec<String>, hdf5::Error> {
    Ok(meta
        .member_names()?
        .into_iter()
        .filter(|name| name.starts_with(STAGE_PREFIX))
        .collect())
}

pub(crate) fn validate_pending_renames(
    file: &File,
    schema_version: i64,
) -> Result<bool, InternalError> {
    let guarded = schema_version == RENAME_SCHEMA_VERSION;
    let meta = match file.group("meta") {
        Ok(meta) => meta,
        Err(_) if guarded => return Err(self::guarded_schema_error().into()),
        Err(_) => return Ok(false),
    };
    let names = self::pending_stage_names(&meta)?;
    if guarded && names.len() != 1 {
        return Err(self::guarded_schema_error().into());
    }
    for name in &names {
        let stage = self::read_stage(file, &meta, name, RenameResolution::Recover)?;
        if guarded && !stage.is_some_and(|stage| stage.schema == Some(RENAME_SCHEMA_VERSION)) {
            return Err(self::guarded_schema_error().into());
        }
    }
    Ok(!names.is_empty())
}

pub(super) fn recover_stage(
    file: &File,
    meta: &Group,
    name: &str,
    before_link_change: impl FnOnce() -> std::io::Result<()>,
) -> Result<(), InternalError> {
    self::settle_stage(
        file,
        meta,
        name,
        RenameResolution::Recover,
        before_link_change,
    )
}

fn settle_stage(
    file: &File,
    meta: &Group,
    name: &str,
    resolution: RenameResolution,
    before_link_change: impl FnOnce() -> std::io::Result<()>,
) -> Result<(), InternalError> {
    let Some(RenameStage {
        recording_name,
        recording,
        source,
        destination,
        schema,
        previous_schema,
        previous_identity,
    }) = self::read_stage(file, meta, name, resolution)?
    else {
        meta.unlink(name)?;
        file.flush()?;
        return Ok(());
    };
    let guarded = schema == Some(RENAME_SCHEMA_VERSION);
    if !guarded {
        if schema == Some(0) && previous_schema.is_none() {
            self::write_schema_version(file, None)?;
        }
        meta.unlink(name)?;
        file.flush()?;
        return Ok(());
    }
    let publish_destination = matches!(resolution, RenameResolution::Commit)
        || previous_identity.as_deref() == Some(destination.identity.as_str());
    let (keep, remove) = if publish_destination {
        (destination, source)
    } else {
        (source, destination)
    };
    before_link_change()?;
    if remove.group.link_exists(recording_name) {
        remove.group.unlink(recording_name)?;
        file.flush()?;
    }
    if previous_identity.as_deref() != Some(keep.identity.as_str()) {
        self::overwrite_identity_attr(&recording, &keep.identity)?;
        file.flush()?;
    }
    if !keep.group.link_exists(recording_name) {
        keep.group.link_hard(&recording.name(), recording_name)?;
        file.flush()?;
    }
    self::write_schema_version(file, previous_schema)?;
    meta.unlink(name)?;
    file.flush()?;
    Ok(())
}

fn read_stage<'a>(
    file: &File,
    meta: &Group,
    name: &'a str,
    resolution: RenameResolution,
) -> Result<Option<RenameStage<'a>>, InternalError> {
    self::require_unique_hard_group_link(file, "meta", meta)?;
    let recording_name = name
        .strip_prefix(STAGE_PREFIX)
        .filter(|name| gt_history_types::is_history_link_name(name))
        .ok_or_else(|| InternalError::Conflict(format!("Invalid History rename stage {name:?}")))?;
    let stage = meta.group(name)?;
    self::require_unique_hard_group_link(meta, name, &stage)?;
    let schema = self::schema_attribute(file)?
        .as_ref()
        .map(self::read_integer_schema)
        .transpose()?;
    let guarded = schema == Some(RENAME_SCHEMA_VERSION);
    match self::link_type(&stage, STAGE_RECORDING)? {
        None if guarded => return Err(self::guarded_schema_error().into()),
        None => return Ok(None),
        Some(LinkType::Hard) => {}
        Some(_) => {
            return Err(InternalError::Conflict(format!(
                "History rename stage {name:?} requires a recording hard link"
            )));
        }
    }
    let previous_schema = match stage.attr(STAGE_PREVIOUS_SCHEMA)?.read_scalar::<i64>()? {
        ABSENT_SCHEMA_VERSION => None,
        version if (0..=CURRENT_SCHEMA_VERSION).contains(&version) => Some(version),
        _ => return Err(self::guarded_schema_error().into()),
    };
    let recording = stage.group(STAGE_RECORDING)?;
    if matches!(resolution, RenameResolution::Recover)
        && super::read_recording_meta(&recording).is_none()
    {
        return Err(InternalError::Conflict(format!(
            "History rename stage {name:?} requires recording metadata"
        )));
    }
    let recorded_object = recording.loc_info()?;
    let by_id = self::direct_root_group(file, "by_identity")?.ok_or_else(|| {
        InternalError::Conflict("History rename requires an identity group".to_owned())
    })?;
    let endpoint = |attribute| -> Result<_, InternalError> {
        let path = super::read_group_string_attr(&stage, attribute)?;
        let identity_name = path
            .strip_prefix("/by_identity/")
            .filter(|name| gt_history_types::is_history_link_name(name))
            .ok_or_else(|| {
                InternalError::Conflict(format!("Invalid History rename endpoint {path:?}"))
            })?;
        let group = by_id.group(identity_name)?;
        if matches!(resolution, RenameResolution::Recover) {
            self::require_unique_hard_group_link(&by_id, identity_name, &group)?;
            if group.link_exists(recording_name)
                && (self::link_type(&group, recording_name)? != Some(LinkType::Hard)
                    || !self::same_object(
                        &group.group(recording_name)?.loc_info()?,
                        &recorded_object,
                    ))
            {
                return Err(InternalError::Conflict(format!(
                    "History rename endpoint {path:?}/{recording_name:?} has an unrelated link"
                )));
            }
        }
        let identity = super::identity_for_group(&group, identity_name);
        if matches!(resolution, RenameResolution::Recover)
            && (RenameIdentityPath {
                identity: &identity,
                storage_group: identity_name,
            })
            .checked_alternate_groups(&by_id)?
            .iter()
            .any(|group| group.link_exists(recording_name))
        {
            return Err(InternalError::Conflict(format!(
                "Recording {path:?}/{recording_name:?} exists in both identity groups"
            )));
        }
        Ok(RenameEndpoint { group, identity })
    };
    let source = endpoint(STAGE_SOURCE)?;
    let destination = endpoint(STAGE_DESTINATION)?;
    if source.identity == destination.identity {
        return Err(InternalError::Conflict(format!(
            "History rename stage {name:?} repeats an identity"
        )));
    }
    if matches!(resolution, RenameResolution::Recover) {
        self::require_unreserved_rename(meta, |pending| {
            pending.group_name == recording_name
                && (pending.identity == source.identity || pending.identity == destination.identity)
        })?;
        let known_links = 1
            + usize::from(source.group.link_exists(recording_name))
            + usize::from(destination.group.link_exists(recording_name));
        if recorded_object.num_links != known_links {
            return Err(InternalError::Conflict(format!(
                "History rename stage {name:?} has additional recording links"
            )));
        }
    }
    let previous_identity = super::recording_identity(&recording);
    if !guarded {
        let source_link = source.group.link_exists(recording_name);
        let destination_link = destination.group.link_exists(recording_name);
        let complete_source = previous_identity
            .as_deref()
            .is_none_or(|identity| identity == source.identity);
        let complete_destination =
            previous_identity.as_deref() == Some(destination.identity.as_str());
        let valid_public_link = (source_link && !destination_link && complete_source)
            || (!source_link && destination_link && complete_destination);
        if !valid_public_link {
            return Err(InternalError::Conflict(format!(
                "History rename stage {name:?} requires schema protection"
            )));
        }
    }
    Ok(Some(RenameStage {
        recording_name,
        recording,
        source,
        destination,
        schema,
        previous_schema,
        previous_identity,
    }))
}

pub(super) fn guarded_schema_error() -> DbError {
    DbError::SchemaTooNew {
        found: RENAME_SCHEMA_VERSION,
        supported: CURRENT_SCHEMA_VERSION,
    }
}

fn schema_attribute(file: &File) -> Result<Option<Attribute>, hdf5::Error> {
    file.attr_names()?
        .iter()
        .any(|name| name == SCHEMA_VERSION_ATTR)
        .then(|| file.attr(SCHEMA_VERSION_ATTR))
        .transpose()
}

fn read_integer_schema(attribute: &Attribute) -> Result<i64, InternalError> {
    match attribute.dtype()?.to_descriptor()? {
        TypeDescriptor::Integer(_) | TypeDescriptor::Unsigned(_) => Ok(attribute.read_scalar()?),
        _ => Err(InternalError::Conflict(
            "History schema version requires an integer attribute".to_owned(),
        )),
    }
}

fn write_schema_version(file: &File, version: Option<i64>) -> Result<(), InternalError> {
    let attribute = self::schema_attribute(file)?;
    if let Some(version) = version {
        let attribute = match attribute {
            Some(attribute) => attribute,
            None => file.new_attr::<i64>().create(SCHEMA_VERSION_ATTR)?,
        };
        attribute.write_scalar(&version)?;
        if attribute.read_scalar::<i64>()? != version {
            return Err(InternalError::Conflict(
                "History schema version cannot be written exactly".to_owned(),
            ));
        }
    } else if attribute.is_some() {
        file.delete_attr(SCHEMA_VERSION_ATTR)?;
    }
    file.flush()?;
    Ok(())
}

fn require_unique_hard_group_link(
    parent: &Group,
    name: &str,
    group: &Group,
) -> Result<(), InternalError> {
    if self::is_unique_hard_group_link(parent, name, group)? {
        Ok(())
    } else {
        Err(InternalError::Conflict(format!(
            "Group {name:?} requires a single hard link"
        )))
    }
}

fn direct_root_group(file: &File, name: &str) -> Result<Option<Group>, InternalError> {
    match self::link_type(file, name)? {
        None => return Ok(None),
        Some(LinkType::Hard) => {
            let group = file.group(name)?;
            if group.loc_info()?.num_links == 1 {
                return Ok(Some(group));
            }
        }
        Some(_) => {}
    }
    Err(InternalError::Conflict(format!(
        "Group {name:?} requires a single hard link"
    )))
}

fn is_unique_hard_group_link(
    parent: &Group,
    name: &str,
    group: &Group,
) -> Result<bool, hdf5::Error> {
    Ok(self::link_type(parent, name)? == Some(LinkType::Hard) && group.loc_info()?.num_links == 1)
}

fn link_type(parent: &Group, name: &str) -> Result<Option<LinkType>, hdf5::Error> {
    parent.find_link(IndexType::Name, IterationOrder::Increasing, |link, info| {
        Ok((link == name).then_some(info.link_type))
    })
}

fn same_object(first: &LocationInfo, second: &LocationInfo) -> bool {
    first.token == second.token && first.fileno == second.fileno
}

pub(crate) fn recording_has_independent_links(
    by_id: &Group,
    recording: &DatabaseRef,
) -> Result<bool, InternalError> {
    let Some((identity_name, identity)) =
        super::find_identity_group(by_id, &recording.identity, Some(&recording.group_name))?
    else {
        return Ok(false);
    };
    let group = identity.group(&recording.group_name)?;
    Ok(
        self::is_unique_hard_group_link(by_id, &identity_name, &identity)?
            && self::is_unique_hard_group_link(&identity, &recording.group_name, &group)?,
    )
}
