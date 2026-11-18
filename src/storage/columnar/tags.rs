//! Tags of a columnar collection (card #981): a name for the collection as
//! it is at an instant, read later as it was, and restored.
//!
//! A tag holds no rows. Blocks are never changed in place: a write that
//! replaces or removes one *retires* it, and a block that a tag names is
//! copied first, once per epoch, under `col_kept:{block}:{epoch}`
//! ([`ColumnarCollection::retire`], called by every path that retires a
//! block). The tail, which writes change in place, is sealed into a block of
//! the tag's own when the tag is taken (`col_kept:tail:{tag}`).
//!
//! Taking a tag moves the collection's `block_epoch`; the tag keeps the new
//! value. For each of its blocks, a tag reads the first copy kept at or
//! after its epoch (the version that was current when it was taken, since
//! that is the version the first retirement after it copied), else the block
//! as stored: it has not been retired since.
//!
//! Tags are local to a node: they are not replicated, not in a full sync,
//! not dumped.
//!
//! A *branch* is a columnar collection that starts from a tag of another
//! one (its parent), read in place: its blocks are its own and those of
//! the tag it has not written. A write that would replace or remove one of
//! the tag's blocks in the branch writes the branch's version and marks
//! the tag's as detached (`col_detached:{block}`, in [`retire`], which
//! every such write calls). The parent lists its branches
//! (`col_branch:{branch}`), which keeps their tags and the parent itself
//! from being dropped. A branch's writes are not replicated.

use super::*;

/// What `col_tag:{tag}` holds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct TagMeta {
    pub tag: String,
    pub created_at: i64,
    /// The collection's `block_epoch` once the tag was taken.
    pub epoch: u64,
    pub columns: Vec<ColumnDef>,
    pub compression: CompressionType,
    pub row_count: u64,
    /// The keys of the blocks the collection held, in order.
    pub blocks: Vec<String>,
    /// Whether the tail held rows, sealed under `col_kept:tail:{tag}`.
    pub tail: bool,
    /// The key of that sealed block (its first row's id).
    #[serde(default)]
    pub tail_block: Option<String>,
}

/// A tag as it is listed.
#[derive(Debug, Clone, Serialize)]
pub struct TagInfo {
    pub tag: String,
    pub created_at: i64,
    pub row_count: u64,
    pub blocks: usize,
}

impl From<&TagMeta> for TagInfo {
    fn from(meta: &TagMeta) -> Self {
        TagInfo {
            tag: meta.tag.clone(),
            created_at: meta.created_at,
            row_count: meta.row_count,
            blocks: meta.blocks.len() + usize::from(meta.tail),
        }
    }
}

/// What a handle that reads a tag reads.
pub(crate) struct TagView {
    pub blocks: Vec<Arc<ColBlock>>,
}

/// What a branch's meta says of where it starts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BranchOf {
    pub collection: String,
    pub tag: String,
    /// The tag was taken for the branch, and goes with it.
    pub implicit: bool,
}

/// What a branch reads of its parent: the tag's blocks, at its epoch.
pub(crate) struct Branch {
    parent: ColumnarCollection,
    tag: TagMeta,
}

impl Branch {
    /// The keys of the tag's blocks, its tail's included.
    pub(crate) fn blocks(&self) -> impl Iterator<Item = &String> {
        self.tag.blocks.iter().chain(self.tag.tail_block.iter())
    }
}

fn tag_key(tag: &str) -> String {
    format!("{COL_TAG_PREFIX}{tag}")
}

fn kept_key(block: &str, epoch: u64) -> String {
    format!("{COL_KEPT_PREFIX}{block}:{epoch:016x}")
}

fn kept_prefix(block: &str) -> String {
    format!("{COL_KEPT_PREFIX}{block}:")
}

/// A tag's sealed tail. `tail:` cannot begin a block key, which is a UUID.
fn tail_key(tag: &str) -> String {
    format!("{COL_KEPT_PREFIX}tail:{tag}")
}

fn block_key(block: &str) -> String {
    format!("{COL_BLOCK_PREFIX}{block}")
}

/// A tag's name: what a URL and an SDBQL string carry without escaping.
fn check_name(tag: &str) -> DbResult<()> {
    let fits = !tag.is_empty()
        && tag.len() <= 128
        && tag
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
    if fits {
        Ok(())
    } else {
        Err(DbError::BadRequest(format!(
            "A tag name is 1 to 128 letters, digits, '_', '-' or '.': '{tag}'"
        )))
    }
}

fn internal(e: impl std::fmt::Display) -> DbError {
    DbError::InternalError(e.to_string())
}

impl ColumnarCollection {
    /// The tags of the collection.
    pub(super) fn tags_of(&self, cf: &KsCf) -> DbResult<Vec<TagMeta>> {
        let prefix = COL_TAG_PREFIX.as_bytes();
        let mut tags = Vec::new();
        for (key, bytes) in self.db.prefix_iterator_ks(cf, prefix).flatten() {
            if !key.starts_with(prefix) {
                break;
            }
            tags.push(serde_json::from_slice::<TagMeta>(&bytes)?);
        }
        Ok(tags)
    }

    /// The blocks some tag names: a write keeps a copy of one before it
    /// retires it ([`Self::retire`]).
    pub(super) fn pinned(&self, cf: &KsCf) -> DbResult<std::collections::HashSet<String>> {
        Ok(self
            .tags_of(cf)?
            .into_iter()
            .flat_map(|tag| tag.blocks)
            .collect())
    }

    /// Before a write replaces or removes the block `block` (staged in
    /// `batch`): when a tag names it, copy it as it is stored under the
    /// write's epoch, unless a copy was kept at that epoch already (a tag
    /// that needs the stored version would have been taken after it, and
    /// moved the epoch).
    pub(super) fn retire(
        &self,
        cf: &KsCf,
        batch: &mut KvBatch,
        pins: &std::collections::HashSet<String>,
        epoch: u64,
        block: &str,
    ) -> DbResult<()> {
        // A branch stops reading its parent's version of the block.
        if let Some(branch) = &self.branch {
            if branch.blocks().any(|b| b == block) {
                batch.put_ks(cf, format!("{COL_DETACHED_PREFIX}{block}").as_bytes(), b"");
            }
        }
        if !pins.contains(block) {
            return Ok(());
        }
        let kept = kept_key(block, epoch);
        if self
            .db
            .get_ks(cf, kept.as_bytes())
            .map_err(internal)?
            .is_some()
        {
            return Ok(());
        }
        if let Some(bytes) = self
            .db
            .get_ks(cf, block_key(block).as_bytes())
            .map_err(internal)?
        {
            batch.put_ks(cf, kept.as_bytes(), bytes);
        }
        Ok(())
    }

    /// The stored bytes of `block` as a tag of epoch `epoch` reads it: the
    /// first copy kept at or after that epoch, else the block as stored.
    fn version_of(&self, cf: &KsCf, block: &str, epoch: u64) -> DbResult<Option<Vec<u8>>> {
        let prefix = kept_prefix(block);
        let from = kept_key(block, epoch);
        for (key, bytes) in self.db.prefix_iterator_ks(cf, prefix.as_bytes()).flatten() {
            if !key.starts_with(prefix.as_bytes()) {
                break;
            }
            if &key[..] >= from.as_bytes() {
                return Ok(Some(bytes.to_vec()));
            }
        }
        self.db
            .get_ks(cf, block_key(block).as_bytes())
            .map_err(internal)
    }

    /// Decode stored block bytes.
    fn decode_block(
        &self,
        bytes: &[u8],
        compression: &CompressionType,
        what: &str,
    ) -> DbResult<ColBlock> {
        self.decompress_data(bytes, compression)
            .ok()
            .and_then(ColBlock::decode)
            .ok_or_else(|| {
                DbError::InternalError(format!(
                    "columnar block '{what}' of '{}' cannot be read",
                    self.name
                ))
            })
    }

    /// Take the tag `tag` of the collection as it is now.
    pub fn create_tag(&self, tag: &str) -> DbResult<TagInfo> {
        check_name(tag)?;
        self.take_tag(tag)
    }

    /// [`Self::create_tag`] without the check of the name: a branch's own
    /// tag is named `~{branch}`, which no client can take.
    fn take_tag(&self, tag: &str) -> DbResult<TagInfo> {
        self.refuse_on_view()?;
        self.refuse_on_branch("tagged")?;
        let shared = self.shared_write_lock();
        let _shared_guard = shared.lock();
        let mut meta = self.meta.write().map_err(internal)?;
        let db = &self.db;
        let cf = self.ks.handle(db).ok_or_else(|| self.not_found())?;
        self.refresh_meta(db, &cf, &mut meta);
        if db
            .get_ks(&cf, tag_key(tag).as_bytes())
            .map_err(internal)?
            .is_some()
        {
            return Err(DbError::ConflictError(format!(
                "Columnar collection '{}' already has a tag '{tag}'",
                self.name
            )));
        }
        let mut batch = KvBatch::default();
        let prefix = COL_BLOCK_PREFIX.as_bytes();
        let mut blocks = Vec::new();
        for (key, _) in db.prefix_iterator_ks(&cf, prefix).flatten() {
            if !key.starts_with(prefix) {
                break;
            }
            blocks.push(String::from_utf8_lossy(&key[prefix.len()..]).into_owned());
        }
        let tail = self.tail(&cf, &meta.compression)?;
        let mut tail_block = None;
        if !tail.is_empty() {
            let names: Vec<String> = meta.columns.iter().map(|c| c.name.clone()).collect();
            let sealed = ColBlock::seal(&names, &tail);
            tail_block = Some(sealed.key.clone());
            let bytes = sealed
                .encode(matches!(meta.compression, CompressionType::Lz4))
                .ok_or_else(|| internal("columnar block: encoding"))?;
            batch.put_ks(
                &cf,
                tail_key(tag).as_bytes(),
                self.compress_data(&bytes, &meta.compression),
            );
        }
        let mut new_meta = meta.clone();
        new_meta.block_epoch += 1;
        let tagged = TagMeta {
            tag: tag.to_string(),
            created_at: chrono::Utc::now().timestamp(),
            epoch: new_meta.block_epoch,
            columns: meta.columns.clone(),
            compression: meta.compression.clone(),
            row_count: meta.row_count,
            blocks,
            tail: !tail.is_empty(),
            tail_block,
        };
        batch.put_ks(&cf, tag_key(tag).as_bytes(), serde_json::to_vec(&tagged)?);
        self.stage_meta(&cf, &mut batch, &new_meta)?;
        db.write(batch).map_err(internal)?;
        *meta = new_meta;
        Ok(TagInfo::from(&tagged))
    }

    /// The tags, by name.
    pub fn list_tags(&self) -> DbResult<Vec<TagInfo>> {
        let cf = self.ks.handle(&self.db).ok_or_else(|| self.not_found())?;
        Ok(self.tags_of(&cf)?.iter().map(TagInfo::from).collect())
    }

    /// Drop the tag `tag`, and the copies no other tag reads.
    pub fn drop_tag(&self, tag: &str) -> DbResult<()> {
        self.refuse_on_view()?;
        let shared = self.shared_write_lock();
        let _shared_guard = shared.lock();
        let db = &self.db;
        let cf = self.ks.handle(db).ok_or_else(|| self.not_found())?;
        let tags = self.tags_of(&cf)?;
        if !tags.iter().any(|t| t.tag == tag) {
            return Err(self.no_tag(tag));
        }
        if let Some((branch, _)) = self.branches_of(&cf)?.into_iter().find(|(_, t)| t == tag) {
            return Err(DbError::ConflictError(format!(
                "Tag '{tag}' of '{}' is read by the branch '{branch}': drop the branch first",
                self.name
            )));
        }
        let mut batch = KvBatch::default();
        batch.delete_ks(&cf, tag_key(tag).as_bytes());
        batch.delete_ks(&cf, tail_key(tag).as_bytes());
        // The copies each tag left reads: for each of its blocks, the first
        // kept at or after its epoch.
        let mut copies: HashMap<String, Vec<(u64, Vec<u8>)>> = HashMap::new();
        let prefix = COL_KEPT_PREFIX.as_bytes();
        let tails = tail_key("");
        for (key, _) in db.prefix_iterator_ks(&cf, prefix).flatten() {
            if !key.starts_with(prefix) {
                break;
            }
            if key.starts_with(tails.as_bytes()) {
                continue;
            }
            let text = String::from_utf8_lossy(&key[prefix.len()..]).into_owned();
            if let Some((block, epoch)) = text.rsplit_once(':') {
                if let Ok(epoch) = u64::from_str_radix(epoch, 16) {
                    copies
                        .entry(block.to_string())
                        .or_default()
                        .push((epoch, key.to_vec()));
                }
            }
        }
        let mut read: std::collections::HashSet<Vec<u8>> = std::collections::HashSet::new();
        for other in tags.iter().filter(|t| t.tag != tag) {
            for block in &other.blocks {
                if let Some(kept) = copies.get(block) {
                    // Keys sort by epoch (fixed-width hex).
                    if let Some((_, key)) = kept.iter().find(|(epoch, _)| *epoch >= other.epoch) {
                        read.insert(key.clone());
                    }
                }
            }
        }
        for kept in copies.values().flatten() {
            if !read.contains(&kept.1) {
                batch.delete_ks(&cf, &kept.1);
            }
        }
        db.write(batch).map_err(internal)?;
        Ok(())
    }

    /// A handle that reads the collection as the tag `tag` holds it: its
    /// blocks, no tail, no index; never written.
    pub fn at_tag(&self, tag: &str) -> DbResult<Self> {
        let db = &self.db;
        let cf = self.ks.handle(db).ok_or_else(|| self.not_found())?;
        let tagged: TagMeta = match db.get_ks(&cf, tag_key(tag).as_bytes()).map_err(internal)? {
            Some(bytes) => serde_json::from_slice(&bytes)?,
            None => return Err(self.no_tag(tag)),
        };
        let mut blocks = Vec::with_capacity(tagged.blocks.len() + 1);
        for block in &tagged.blocks {
            let bytes = self.version_of(&cf, block, tagged.epoch)?.ok_or_else(|| {
                DbError::InternalError(format!(
                    "block '{block}' of tag '{tag}' of '{}' is missing",
                    self.name
                ))
            })?;
            blocks.push(Arc::new(self.decode_block(
                &bytes,
                &tagged.compression,
                block,
            )?));
        }
        if tagged.tail {
            let bytes = db
                .get_ks(&cf, tail_key(tag).as_bytes())
                .map_err(internal)?
                .ok_or_else(|| internal(format!("the tail of tag '{tag}' is missing")))?;
            blocks.push(Arc::new(self.decode_block(
                &bytes,
                &tagged.compression,
                "tail",
            )?));
        }
        blocks.sort_by(|a, b| a.key.cmp(&b.key));
        let mut meta = self.meta.read().map_err(internal)?.clone();
        meta.columns = tagged.columns;
        meta.compression = tagged.compression;
        meta.row_count = tagged.row_count;
        meta.tail_rows = 0;
        meta.index_rows.clear();
        meta.index_base.clear();
        meta.index_builds.clear();
        Ok(Self {
            name: self.name.clone(),
            db: self.db.clone(),
            cf_name: self.cf_name.clone(),
            ks: self.ks.clone(),
            meta: Arc::new(RwLock::new(meta)),
            at: Some(Arc::new(TagView { blocks })),
            branch: None,
        })
    }

    /// Make the tag `tag` the collection's rows again: every block and row
    /// the collection holds now goes (kept for the tags that name it), and
    /// the tag's come back, with its schema. The indexes answer for no row
    /// afterwards: they are built again on demand. Returns the rows now
    /// held.
    pub fn restore_tag(&self, tag: &str) -> DbResult<u64> {
        self.refuse_on_view()?;
        self.refuse_on_branch("restored")?;
        let shared = self.shared_write_lock();
        let _shared_guard = shared.lock();
        let _wrote = self.wrote_on_release();
        let mut meta = self.meta.write().map_err(internal)?;
        let db = &self.db;
        let cf = self.ks.handle(db).ok_or_else(|| self.not_found())?;
        self.refresh_meta(db, &cf, &mut meta);
        let tags = self.tags_of(&cf)?;
        let Some(tagged) = tags.iter().find(|t| t.tag == tag) else {
            return Err(self.no_tag(tag));
        };
        let pins: std::collections::HashSet<String> =
            tags.iter().flat_map(|t| t.blocks.iter().cloned()).collect();
        let mut batch = KvBatch::default();
        // What is there now goes. Reads below see the store as it is before
        // this batch.
        let prefix = COL_BLOCK_PREFIX.as_bytes();
        let mut current = Vec::new();
        for (key, _) in db.prefix_iterator_ks(&cf, prefix).flatten() {
            if !key.starts_with(prefix) {
                break;
            }
            current.push(String::from_utf8_lossy(&key[prefix.len()..]).into_owned());
        }
        let mut versions = Vec::with_capacity(tagged.blocks.len());
        for block in &tagged.blocks {
            let bytes = self.version_of(&cf, block, tagged.epoch)?.ok_or_else(|| {
                DbError::InternalError(format!(
                    "block '{block}' of tag '{tag}' of '{}' is missing",
                    self.name
                ))
            })?;
            versions.push((block.clone(), bytes));
        }
        for block in &current {
            self.retire(&cf, &mut batch, &pins, meta.block_epoch, block)?;
            batch.delete_ks(&cf, block_key(block).as_bytes());
        }
        let rows = COL_ROW_PREFIX.as_bytes();
        for (key, _) in db.prefix_iterator_ks(&cf, rows).flatten() {
            if !key.starts_with(rows) {
                break;
            }
            batch.delete_ks(&cf, &key);
        }
        // Then the tag's, after the deletes in the batch.
        for (block, bytes) in versions {
            batch.put_ks(&cf, block_key(&block).as_bytes(), bytes);
        }
        if tagged.tail {
            let bytes = db
                .get_ks(&cf, tail_key(tag).as_bytes())
                .map_err(internal)?
                .ok_or_else(|| internal(format!("the tail of tag '{tag}' is missing")))?;
            let sealed = self.decode_block(&bytes, &tagged.compression, "tail")?;
            batch.put_ks(&cf, block_key(&sealed.key).as_bytes(), bytes);
        }
        let mut new_meta = meta.clone();
        new_meta.columns = tagged.columns.clone();
        new_meta.compression = tagged.compression.clone();
        new_meta.row_count = tagged.row_count;
        new_meta.tail_rows = 0;
        for held in new_meta.index_rows.values_mut() {
            *held = 0;
        }
        new_meta.index_base.clear();
        new_meta.last_updated_at = chrono::Utc::now().timestamp();
        self.stage_meta(&cf, &mut batch, &new_meta)?;
        self.note_moved();
        db.write(batch).map_err(internal)?;
        *meta = new_meta;
        *self.store().blocks.write() = None;
        Ok(tagged.row_count)
    }

    fn no_tag(&self, tag: &str) -> DbError {
        DbError::DocumentNotFound(format!(
            "Columnar collection '{}' has no tag '{tag}'",
            self.name
        ))
    }

    fn refuse_on_branch(&self, what: &str) -> DbResult<()> {
        if self.branch.is_some() {
            return Err(DbError::OperationNotSupported(format!(
                "'{}' is a branch, which is not {what}",
                self.name
            )));
        }
        Ok(())
    }

    /// Take the tag a branch named `branch` starts from, when none is
    /// given: `~{branch}`, a name no client can take. Returns its name.
    pub fn take_branch_tag(&self, branch: &str) -> DbResult<String> {
        let tag = format!("~{branch}");
        self.take_tag(&tag)?;
        Ok(tag)
    }

    /// The columns, compression and rows of the tag `tag`: what a branch
    /// starting from it is created with.
    pub fn tag_start(&self, tag: &str) -> DbResult<(Vec<ColumnDef>, CompressionType, u64)> {
        let cf = self.ks.handle(&self.db).ok_or_else(|| self.not_found())?;
        let tagged: TagMeta = match self
            .db
            .get_ks(&cf, tag_key(tag).as_bytes())
            .map_err(internal)?
        {
            Some(bytes) => serde_json::from_slice(&bytes)?,
            None => return Err(self.no_tag(tag)),
        };
        Ok((tagged.columns, tagged.compression, tagged.row_count))
    }

    /// Make this new, empty collection a branch of `parent` at `of.tag`:
    /// its meta names the parent and counts the tag's rows, and the parent
    /// lists it.
    pub fn attach_to(&self, parent: &ColumnarCollection, of: BranchOf, rows: u64) -> DbResult<()> {
        let cf = self.ks.handle(&self.db).ok_or_else(|| self.not_found())?;
        let pcf = parent
            .ks
            .handle(&parent.db)
            .ok_or_else(|| parent.not_found())?;
        let mut meta = self.meta.write().map_err(internal)?;
        let mut new_meta = meta.clone();
        new_meta.parent = Some(of.clone());
        new_meta.row_count = rows;
        let mut batch = KvBatch::default();
        batch.put_ks(
            &pcf,
            format!("{COL_BRANCH_PREFIX}{}", self.name).as_bytes(),
            of.tag.as_bytes(),
        );
        self.stage_meta(&cf, &mut batch, &new_meta)?;
        self.db.write(batch).map_err(internal)?;
        *meta = new_meta;
        // Kept as the empty collection it was: read again, with the tag's.
        *self.store().blocks.write() = None;
        Ok(())
    }

    /// After this branch was dropped: its parent no longer lists it, and a
    /// tag taken for it goes.
    pub fn detach_from_parent(&self) -> DbResult<()> {
        let Some(branch) = &self.branch else {
            return Ok(());
        };
        let of = self
            .meta
            .read()
            .map_err(internal)?
            .parent
            .clone()
            .ok_or_else(|| internal("a branch without a parent"))?;
        let parent = &branch.parent;
        let pcf = parent
            .ks
            .handle(&parent.db)
            .ok_or_else(|| parent.not_found())?;
        parent
            .db
            .delete_ks(&pcf, format!("{COL_BRANCH_PREFIX}{}", self.name).as_bytes())
            .map_err(internal)?;
        if of.implicit {
            parent.drop_tag(&of.tag)?;
        }
        Ok(())
    }

    /// The branches of this collection and the tag each reads.
    pub(crate) fn branches_of(&self, cf: &KsCf) -> DbResult<Vec<(String, String)>> {
        let prefix = COL_BRANCH_PREFIX.as_bytes();
        let mut found = Vec::new();
        for (key, bytes) in self.db.prefix_iterator_ks(cf, prefix).flatten() {
            if !key.starts_with(prefix) {
                break;
            }
            found.push((
                String::from_utf8_lossy(&key[prefix.len()..]).into_owned(),
                String::from_utf8_lossy(&bytes).into_owned(),
            ));
        }
        Ok(found)
    }

    /// The names of this collection's branches.
    pub fn branches(&self) -> DbResult<Vec<String>> {
        let cf = self.ks.handle(&self.db).ok_or_else(|| self.not_found())?;
        Ok(self.branches_of(&cf)?.into_iter().map(|(b, _)| b).collect())
    }

    /// Whether this collection is a branch.
    pub fn is_branch(&self) -> bool {
        self.branch.is_some()
    }

    /// What a branch reads of its parent, from its meta (`None` for a
    /// collection that is not one). Called when the collection is loaded.
    pub(super) fn open_branch(&self) -> DbResult<Option<Arc<Branch>>> {
        let Some(of) = self.meta.read().map_err(internal)?.parent.clone() else {
            return Ok(None);
        };
        let db_name = self
            .cf_name
            .split_once(':')
            .map_or("", |(db, _)| db)
            .to_string();
        let parent = ColumnarCollection::load(of.collection.clone(), &db_name, self.db.clone())?;
        let found = {
            let pcf = parent
                .ks
                .handle(&parent.db)
                .ok_or_else(|| parent.not_found())?;
            parent
                .db
                .get_ks(&pcf, tag_key(&of.tag).as_bytes())
                .map_err(internal)?
        };
        let tag: TagMeta = match found {
            Some(bytes) => serde_json::from_slice(&bytes)?,
            None => return Err(parent.no_tag(&of.tag)),
        };
        Ok(Some(Arc::new(Branch { parent, tag })))
    }

    /// The tag's blocks a branch still reads in place: those it has not
    /// detached, as its parent holds them at the tag's epoch.
    pub(super) fn shared_blocks(&self, cf: &KsCf, branch: &Branch) -> DbResult<Vec<Arc<ColBlock>>> {
        let parent = &branch.parent;
        let pcf = parent
            .ks
            .handle(&parent.db)
            .ok_or_else(|| parent.not_found())?;
        let mut blocks = Vec::new();
        for block in &branch.tag.blocks {
            let detached = self
                .db
                .get_ks(cf, format!("{COL_DETACHED_PREFIX}{block}").as_bytes())
                .map_err(internal)?
                .is_some();
            if detached {
                continue;
            }
            let bytes = parent
                .version_of(&pcf, block, branch.tag.epoch)?
                .ok_or_else(|| {
                    internal(format!("block '{block}' of '{}' is missing", parent.name))
                })?;
            blocks.push(Arc::new(parent.decode_block(
                &bytes,
                &branch.tag.compression,
                block,
            )?));
        }
        if let Some(key) = &branch.tag.tail_block {
            let tail = format!("{COL_DETACHED_PREFIX}{key}");
            if self
                .db
                .get_ks(cf, tail.as_bytes())
                .map_err(internal)?
                .is_none()
            {
                let bytes = parent
                    .db
                    .get_ks(&pcf, tail_key(&branch.tag.tag).as_bytes())
                    .map_err(internal)?
                    .ok_or_else(|| internal("the tail of a branch's tag is missing"))?;
                let block = parent.decode_block(&bytes, &branch.tag.compression, "tail")?;
                blocks.push(Arc::new(block));
            }
        }
        Ok(blocks)
    }

    /// A handle that reads a tag is never written.
    fn refuse_on_view(&self) -> DbResult<()> {
        if self.at.is_some() {
            return Err(DbError::OperationNotSupported(
                "A tag of a columnar collection is read, not written".to_string(),
            ));
        }
        Ok(())
    }
}
