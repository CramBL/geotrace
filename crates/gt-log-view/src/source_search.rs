use std::ops::Range;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;

use gt_logfile::ParsedLog;
use gt_ui_types::LoadedLogId;
use parking_lot::Mutex;
use regex::RegexBuilder;

use crate::{LoadedLogs, VisibleEntries};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SourceSearchScope {
    SearchFilteredEntries,
    #[default]
    SearchWholeLog,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SourceSearchStatus {
    AwaitFilters,
    AwaitSearch,
    ReportEmptyQuery,
    ReportFailure(String),
    ReportUnavailableLog,
    ShowResults,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceOccurrence {
    log: LoadedLogId,
    line_number: u32,
    byte_range: Range<usize>,
}

impl SourceOccurrence {
    pub fn log(&self) -> LoadedLogId {
        self.log
    }

    pub fn line_number(&self) -> u32 {
        self.line_number
    }

    /// Byte offsets in the original physical source line.
    pub fn byte_range(&self) -> Range<usize> {
        self.byte_range.clone()
    }

    pub fn source_text<'a>(&self, logs: &'a LoadedLogs) -> Option<&'a str> {
        logs.get_by_id(self.log)?
            .parsed()
            .source_line(self.line_number)
    }

    pub fn matched_text<'a>(&self, logs: &'a LoadedLogs) -> Option<&'a str> {
        self.source_text(logs)?.get(self.byte_range())
    }
}

#[derive(Debug)]
pub struct SourceSearch {
    log: LoadedLogId,
    text: String,
    scope: SourceSearchScope,
    key: Option<SearchKey>,
    shared: Arc<SearchCompletion>,
    status: SourceSearchStatus,
    occurrences: Vec<SourceOccurrence>,
    current_index: Option<usize>,
    pending_anchor: Option<SourceOccurrence>,
}

impl SourceSearch {
    pub fn new(log: LoadedLogId) -> Self {
        Self {
            log,
            text: String::new(),
            scope: SourceSearchScope::default(),
            key: None,
            shared: Arc::new(SearchCompletion::default()),
            status: SourceSearchStatus::ReportEmptyQuery,
            occurrences: Vec::new(),
            current_index: None,
            pending_anchor: None,
        }
    }

    pub fn update(&mut self, logs: &LoadedLogs, text: &str, scope: SourceSearchScope) -> bool {
        let before = self.shared.generation.load(Ordering::SeqCst);
        if let Some(work) = self.request(logs, text, scope) {
            work.spawn(Arc::clone(&self.shared));
        }
        self.take_completed() || before != self.shared.generation.load(Ordering::SeqCst)
    }

    pub fn select_next(&mut self) -> Option<&SourceOccurrence> {
        self.current_index = if self.occurrences.is_empty() {
            None
        } else {
            Some(
                self.current_index
                    .map_or(0, |index| (index + 1) % self.occurrences.len()),
            )
        };
        self.current()
    }

    pub fn select_previous(&mut self) -> Option<&SourceOccurrence> {
        self.current_index = self.occurrences.len().checked_sub(1).map(|last| {
            self.current_index
                .map_or(last, |index| index.checked_sub(1).unwrap_or(last))
        });
        self.current()
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn scope(&self) -> SourceSearchScope {
        self.scope
    }

    pub fn status(&self) -> &SourceSearchStatus {
        &self.status
    }

    pub fn occurrences(&self) -> &[SourceOccurrence] {
        &self.occurrences
    }

    pub fn current_index(&self) -> Option<usize> {
        self.current_index
    }

    pub fn current(&self) -> Option<&SourceOccurrence> {
        self.current_index
            .and_then(|index| self.occurrences.get(index))
    }

    fn request(
        &mut self,
        logs: &LoadedLogs,
        text: &str,
        scope: SourceSearchScope,
    ) -> Option<SearchWork> {
        let key = logs.get_by_id(self.log).map(|log| SearchKey {
            source: Arc::clone(log.parsed().text()),
            filtered: (scope == SourceSearchScope::SearchFilteredEntries).then(|| FilterSnapshot {
                semantic_revision: log.filters().table_semantic_revision(),
                entries: log.filters().shared_visible_entries(),
                pending: log.filters().is_table_query_pending(),
            }),
        });
        if self.text == text
            && self.scope == scope
            && self.key == key
            && (key.is_some() || self.status == SourceSearchStatus::ReportUnavailableLog)
        {
            return None;
        }
        self.pending_anchor = self.current().cloned().or(self.pending_anchor.take());
        self.occurrences.clear();
        self.current_index = None;
        let generation = self.shared.advance();
        self.text = text.to_owned();
        self.scope = scope;
        self.key = key;
        let Some(log) = logs.get_by_id(self.log) else {
            self.pending_anchor = None;
            self.status = SourceSearchStatus::ReportUnavailableLog;
            return None;
        };
        if text.is_empty() {
            self.pending_anchor = None;
            self.status = SourceSearchStatus::ReportEmptyQuery;
            return None;
        }
        if self
            .key
            .as_ref()?
            .filtered
            .as_ref()
            .is_some_and(|snapshot| snapshot.pending)
        {
            self.status = SourceSearchStatus::AwaitFilters;
            return None;
        }
        self.status = SourceSearchStatus::AwaitSearch;
        Some(SearchWork {
            log: self.log,
            parsed: log.filters().shared_parsed_log(),
            entries: self
                .key
                .as_ref()?
                .filtered
                .as_ref()
                .map(|snapshot| Arc::clone(&snapshot.entries)),
            text: text.to_owned(),
            generation,
        })
    }

    fn take_completed(&mut self) -> bool {
        let Some(completed) = self.shared.result.lock().take() else {
            return false;
        };
        if completed.generation != self.shared.generation.load(Ordering::SeqCst) {
            return false;
        }
        match completed.result {
            Ok(occurrences) => {
                self.current_index = self
                    .pending_anchor
                    .take()
                    .and_then(|anchor| occurrences.iter().position(|hit| *hit == anchor))
                    .or_else(|| (!occurrences.is_empty()).then_some(0));
                self.occurrences = occurrences;
                self.status = SourceSearchStatus::ShowResults;
            }
            Err(error) => self.status = SourceSearchStatus::ReportFailure(error),
        }
        true
    }
}

impl Drop for SourceSearch {
    fn drop(&mut self) {
        self.shared.advance();
    }
}

#[derive(Debug)]
struct SearchKey {
    source: Arc<str>,
    filtered: Option<FilterSnapshot>,
}

impl PartialEq for SearchKey {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.source, &other.source) && self.filtered == other.filtered
    }
}

#[derive(Debug)]
struct FilterSnapshot {
    semantic_revision: u64,
    entries: Arc<VisibleEntries>,
    pending: bool,
}

impl PartialEq for FilterSnapshot {
    fn eq(&self, other: &Self) -> bool {
        self.semantic_revision == other.semantic_revision
            && self.pending == other.pending
            && Arc::ptr_eq(&self.entries, &other.entries)
    }
}

#[derive(Debug, Default)]
struct SearchCompletion {
    generation: AtomicU64,
    result: Mutex<Option<CompletedSearch>>,
}

impl SearchCompletion {
    fn advance(&self) -> u64 {
        let mut result = self.result.lock();
        let generation = self
            .generation
            .fetch_add(1, Ordering::SeqCst)
            .wrapping_add(1);
        *result = None;
        generation
    }

    fn complete(&self, generation: u64, result: Result<Vec<SourceOccurrence>, String>) {
        let mut slot = self.result.lock();
        if self.generation.load(Ordering::SeqCst) == generation {
            *slot = Some(CompletedSearch { generation, result });
        }
    }
}

#[derive(Debug)]
struct CompletedSearch {
    generation: u64,
    result: Result<Vec<SourceOccurrence>, String>,
}

struct SearchWork {
    log: LoadedLogId,
    parsed: Arc<ParsedLog>,
    entries: Option<Arc<VisibleEntries>>,
    text: String,
    generation: u64,
}

impl SearchWork {
    fn spawn(self, shared: Arc<SearchCompletion>) {
        let completion = Arc::clone(&shared);
        let generation = self.generation;
        let run = move || self.run(&shared);
        if let Some(pool) = gt_logfile::log_worker_pool() {
            pool.spawn(run);
        } else if let Err(error) = thread::Builder::new()
            .name(SEARCH_THREAD_NAME.to_owned())
            .spawn(run)
        {
            completion.complete(generation, Err(error.to_string()));
        }
    }

    fn run(self, shared: &SearchCompletion) {
        if let Some(result) =
            self.scan(|| shared.generation.load(Ordering::SeqCst) == self.generation)
        {
            shared.complete(self.generation, result);
        }
    }

    fn scan(&self, is_current: impl Fn() -> bool) -> Option<Result<Vec<SourceOccurrence>, String>> {
        if !is_current() {
            return None;
        }
        let pattern = match RegexBuilder::new(&regex::escape(&self.text))
            .case_insensitive(true)
            .build()
        {
            Ok(pattern) => pattern,
            Err(error) => return Some(Err(error.to_string())),
        };
        let mut occurrences = Vec::new();
        let count = self
            .entries
            .as_ref()
            .map_or(self.parsed.source_line_count(), |entries| entries.len());
        for row in 0..count {
            if !is_current() {
                return None;
            }
            let line_number = match &self.entries {
                Some(entries) => entries
                    .entry_index(row)
                    .and_then(|index| self.parsed.entries().get(index))
                    .map(|entry| entry.line_number),
                None => row.checked_add(1).and_then(|line| u32::try_from(line).ok()),
            };
            let Some(line_number) = line_number else {
                return Some(Err(
                    "Source line number exceeds the supported range".to_owned()
                ));
            };
            let Some(source) = self.parsed.source_line(line_number) else {
                continue;
            };
            for found in pattern.find_iter(source) {
                if !is_current() {
                    return None;
                }
                occurrences.push(SourceOccurrence {
                    log: self.log,
                    line_number,
                    byte_range: found.range(),
                });
            }
        }
        Some(Ok(occurrences))
    }
}

const SEARCH_THREAD_NAME: &str = "log-source-search";

#[cfg(test)]
mod tests;
