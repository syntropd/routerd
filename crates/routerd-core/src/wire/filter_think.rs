//! Streaming filter separating reasoning thoughts (`<think>...</think>`) from content.

/// Output token or text slice from the thinking filter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilteredItem {
    Reasoning(String),
    Content(String),
}

/// Parsing state of the thinking filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThinkFilterState {
    Outside,
    InThink,
}

/// State machine that filters `<think>...</think>` tags from streaming text chunks.
pub struct ThinkFilter {
    state: ThinkFilterState,
    pending: String,
}

impl Default for ThinkFilter {
    fn default() -> Self {
        Self::new()
    }
}

impl ThinkFilter {
    pub fn new() -> Self {
        Self {
            state: ThinkFilterState::Outside,
            pending: String::new(),
        }
    }

    pub fn with_state(state: ThinkFilterState) -> Self {
        Self {
            state,
            pending: String::new(),
        }
    }

    pub fn state(&self) -> ThinkFilterState {
        self.state
    }

    /// Process a streaming chunk of text, emitting separated reasoning and content items.
    pub fn process(&mut self, chunk: &str) -> Vec<FilteredItem> {
        let mut items = Vec::new();
        for ch in chunk.chars() {
            self.step_char(ch, &mut items);
        }
        items
    }

    fn step_char(&mut self, ch: char, items: &mut Vec<FilteredItem>) {
        const OPEN_TAG: &str = "<think>";
        const CLOSE_TAG: &str = "</think>";

        match self.state {
            ThinkFilterState::Outside => {
                if self.pending.is_empty() {
                    if ch == '<' {
                        self.pending.push(ch);
                    } else {
                        push_content(items, ch);
                    }
                } else {
                    let mut candidate = self.pending.clone();
                    candidate.push(ch);
                    if OPEN_TAG.starts_with(&candidate) || CLOSE_TAG.starts_with(&candidate) {
                        self.pending = candidate;
                        if self.pending == OPEN_TAG {
                            self.pending.clear();
                            self.state = ThinkFilterState::InThink;
                        } else if self.pending == CLOSE_TAG {
                            self.pending.clear();
                        }
                    } else {
                        let flushed = std::mem::take(&mut self.pending);
                        for old_ch in flushed.chars() {
                            push_content(items, old_ch);
                        }
                        if ch == '<' {
                            self.pending.push(ch);
                        } else {
                            push_content(items, ch);
                        }
                    }
                }
            }
            ThinkFilterState::InThink => {
                if self.pending.is_empty() {
                    if ch == '<' {
                        self.pending.push(ch);
                    } else {
                        push_reasoning(items, ch);
                    }
                } else {
                    let mut candidate = self.pending.clone();
                    candidate.push(ch);
                    if CLOSE_TAG.starts_with(&candidate) {
                        self.pending = candidate;
                        if self.pending == CLOSE_TAG {
                            self.pending.clear();
                            self.state = ThinkFilterState::Outside;
                        }
                    } else {
                        let flushed = std::mem::take(&mut self.pending);
                        for old_ch in flushed.chars() {
                            push_reasoning(items, old_ch);
                        }
                        if ch == '<' {
                            self.pending.push(ch);
                        } else {
                            push_reasoning(items, ch);
                        }
                    }
                }
            }
        }
    }

    /// Flush any remaining buffered characters at the end of the stream.
    pub fn flush(&mut self) -> Vec<FilteredItem> {
        let mut items = Vec::new();
        if !self.pending.is_empty() {
            let leftover = std::mem::take(&mut self.pending);
            match self.state {
                ThinkFilterState::Outside => items.push(FilteredItem::Content(leftover)),
                ThinkFilterState::InThink => items.push(FilteredItem::Reasoning(leftover)),
            }
        }
        items
    }
}

fn push_content(items: &mut Vec<FilteredItem>, ch: char) {
    if let Some(FilteredItem::Content(s)) = items.last_mut() {
        s.push(ch);
    } else {
        items.push(FilteredItem::Content(ch.to_string()));
    }
}

fn push_reasoning(items: &mut Vec<FilteredItem>, ch: char) {
    if let Some(FilteredItem::Reasoning(s)) = items.last_mut() {
        s.push(ch);
    } else {
        items.push(FilteredItem::Reasoning(ch.to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plain_content_without_thinking() {
        let mut filter = ThinkFilter::new();
        let items = filter.process("Hello, world!");
        assert_eq!(items, vec![FilteredItem::Content("Hello, world!".into())]);
        assert!(filter.flush().is_empty());
    }

    #[test]
    fn test_single_chunk_with_think_tags() {
        let mut filter = ThinkFilter::new();
        let items = filter.process("<think>reasoning step</think>final answer");
        assert_eq!(
            items,
            vec![
                FilteredItem::Reasoning("reasoning step".into()),
                FilteredItem::Content("final answer".into()),
            ]
        );
        assert!(filter.flush().is_empty());
    }

    #[test]
    fn test_split_across_streaming_chunks() {
        let mut filter = ThinkFilter::new();
        assert_eq!(filter.process("<th"), vec![]);
        assert_eq!(filter.process("ink>thinking text</th"), vec![FilteredItem::Reasoning("thinking text".into())]);
        assert_eq!(filter.process("ink>content here"), vec![FilteredItem::Content("content here".into())]);
        assert!(filter.flush().is_empty());
    }

    #[test]
    fn test_flush_unmatched_tag_at_eos() {
        let mut filter = ThinkFilter::new();
        assert_eq!(filter.process("prefix <th"), vec![FilteredItem::Content("prefix ".into())]);
        assert_eq!(filter.flush(), vec![FilteredItem::Content("<th".into())]);

        let mut filter2 = ThinkFilter::with_state(ThinkFilterState::InThink);
        assert_eq!(filter2.process("deep thought </th"), vec![FilteredItem::Reasoning("deep thought ".into())]);
        assert_eq!(filter2.flush(), vec![FilteredItem::Reasoning("</th".into())]);
    }

    #[test]
    fn test_false_alarm_tag() {
        let mut filter = ThinkFilter::new();
        let items = filter.process("<thought>not a think tag</thought>");
        assert_eq!(
            items,
            vec![FilteredItem::Content("<thought>not a think tag</thought>".into())]
        );
    }

    #[test]
    fn test_nested_brackets() {
        let mut filter = ThinkFilter::new();
        let items = filter.process("<<think>foo</think>");
        assert_eq!(
            items,
            vec![
                FilteredItem::Content("<".into()),
                FilteredItem::Reasoning("foo".into()),
            ]
        );
    }

    #[test]
    fn test_swallow_leading_end_think_tag() {
        let mut filter = ThinkFilter::new();
        let items = filter.process("</think>Hello world");
        assert_eq!(items, vec![FilteredItem::Content("Hello world".into())]);
        assert!(filter.flush().is_empty());

        let mut filter2 = ThinkFilter::new();
        assert_eq!(filter2.process("</th"), vec![]);
        assert_eq!(filter2.process("ink>Streaming answer"), vec![FilteredItem::Content("Streaming answer".into())]);
        assert!(filter2.flush().is_empty());
    }
}
