//! Native Markdown presentation for Work replies. No HTML or remote resource execution.
use super::super::{icons::UiIcon, theme::*, widgets::*, Action, Companion, WorkAction};
use facloud_ui::{elements::*, fonts::Weight, text_layout::TextAlignment};
use markdown_parser::{
    parse_markdown_with_gfm_tables, FormattedTable, FormattedText, FormattedTextLine, Hyperlink,
    TableAlignment,
};

#[derive(Debug, Clone)]
enum RichBlock {
    Markdown(FormattedText),
    Quote(FormattedText),
}

// Parsing does not depend on the theme. Reuse it during progress updates and
// window resizing, with a strict bound so long conversations cannot grow it forever.
#[derive(Default)]
struct RichTextCache {
    entries: std::collections::VecDeque<(String, std::sync::Arc<[RichBlock]>)>,
    bytes: usize,
}
impl RichTextCache {
    fn get(&mut self, source: &str) -> std::sync::Arc<[RichBlock]> {
        if let Some(index) = self.entries.iter().position(|(text, _)| text == source) {
            if index == 0 {
                return std::sync::Arc::clone(&self.entries[0].1);
            }
            let entry = self.entries.remove(index).unwrap();
            let result = std::sync::Arc::clone(&entry.1);
            self.entries.push_front(entry);
            return result;
        }
        let result: std::sync::Arc<[RichBlock]> = parse_blocks(source).into();
        if source.len() <= 256 * 1024 {
            while self.entries.len() >= 64 || self.bytes + source.len() > 2 * 1024 * 1024 {
                let Some((text, _)) = self.entries.pop_back() else {
                    break;
                };
                self.bytes -= text.len();
            }
            self.bytes += source.len();
            self.entries
                .push_front((source.to_owned(), std::sync::Arc::clone(&result)));
        }
        result
    }
}
thread_local! {
    static RICH_TEXT_CACHE: std::cell::RefCell<RichTextCache> = Default::default();
}

fn safe_link(url: &str) -> bool {
    let prefix = if url.starts_with("https://") {
        8
    } else if url.starts_with("http://") {
        7
    } else {
        return false;
    };
    let authority = url[prefix..].split(['/', '?', '#']).next().unwrap_or("");
    !authority.is_empty() && !url.chars().any(|c| c.is_control() || c.is_whitespace())
}

fn parse_blocks(source: &str) -> Vec<RichBlock> {
    let mut blocks = Vec::new();
    let mut pending = String::new();
    let mut is_quote = false;
    let mut fence: Option<char> = None;
    let flush = |blocks: &mut Vec<RichBlock>, pending: &mut String, quote: bool| {
        if pending.trim().is_empty() {
            pending.clear();
            return;
        }
        let mut value = parse_markdown_with_gfm_tables(pending).unwrap_or_else(|_| {
            FormattedText::new([FormattedTextLine::Line(vec![
                markdown_parser::FormattedTextFragment::plain_text(pending.clone()),
            ])])
        });
        for line in &mut value.lines {
            sanitize_links(line);
        }
        blocks.push(if quote {
            RichBlock::Quote(value)
        } else {
            RichBlock::Markdown(value)
        });
        pending.clear();
    };
    for line in source.lines() {
        let trim = line.trim_start();
        let quoted = fence.is_none() && trim.starts_with('>');
        if quoted != is_quote {
            flush(&mut blocks, &mut pending, is_quote);
            is_quote = quoted;
        }
        if !quoted && (trim.starts_with("```") || trim.starts_with("~~~")) {
            let kind = trim.chars().next().unwrap();
            fence = if fence == Some(kind) {
                None
            } else if fence.is_none() {
                Some(kind)
            } else {
                fence
            };
        }
        pending.push_str(if quoted {
            trim.strip_prefix('>')
                .unwrap()
                .strip_prefix(' ')
                .unwrap_or(&trim[1..])
        } else {
            line
        });
        pending.push('\n');
    }
    flush(&mut blocks, &mut pending, is_quote);
    blocks
}

/// Use exactly the same parser as the displayed reply, including quotes and code fences.
pub(in super::super) fn markdown_images(
    source: &str,
) -> Vec<stepx_companion::openclaw::RealAttachment> {
    let mut seen = std::collections::HashSet::new();
    let blocks = RICH_TEXT_CACHE.with(|cache| cache.borrow_mut().get(source));
    blocks
        .iter()
        .flat_map(|block| match block {
            RichBlock::Markdown(text) | RichBlock::Quote(text) => text.lines.iter(),
        })
        .filter_map(|line| match line {
            FormattedTextLine::Image(image)
                if safe_link(&image.source) && seen.insert(image.source.clone()) =>
            {
                Some(stepx_companion::openclaw::RealAttachment {
                    name: if image.alt_text.is_empty() {
                        "图片".into()
                    } else {
                        image.alt_text.clone()
                    },
                    kind: "image".into(),
                    url: image.source.clone(),
                    ..Default::default()
                })
            }
            _ => None,
        })
        .collect()
}

fn sanitize_fragments(fragments: &mut [markdown_parser::FormattedTextFragment]) {
    for fragment in fragments {
        if fragment
            .styles
            .hyperlink
            .as_ref()
            .is_some_and(|link| match link {
                Hyperlink::Url(url) => !safe_link(url),
                Hyperlink::Action(_) => true,
            })
        {
            fragment.styles.hyperlink = None;
        } else if fragment.styles.hyperlink.is_some() {
            fragment.styles.underline = true;
        }
    }
}

fn sanitize_links(line: &mut FormattedTextLine) {
    match line {
        FormattedTextLine::Line(text) => sanitize_fragments(text),
        FormattedTextLine::Heading(header) => sanitize_fragments(&mut header.text),
        FormattedTextLine::OrderedList(list) => sanitize_fragments(&mut list.indented_text.text),
        FormattedTextLine::UnorderedList(list) => sanitize_fragments(&mut list.text),
        FormattedTextLine::TaskList(list) => sanitize_fragments(&mut list.text),
        FormattedTextLine::Table(table) => {
            for cell in &mut table.headers {
                sanitize_fragments(cell);
            }
            for row in &mut table.rows {
                for cell in row {
                    sanitize_fragments(cell);
                }
            }
        }
        _ => {}
    }
}

fn formatted_line_char_count(line: &FormattedTextLine) -> usize {
    match line {
        FormattedTextLine::Heading(header) => {
            header.text.iter().map(|f| f.text.chars().count()).sum()
        }
        FormattedTextLine::Line(fragments) => {
            fragments.iter().map(|f| f.text.chars().count()).sum()
        }
        FormattedTextLine::TaskList(task) => {
            task.text.iter().map(|f| f.text.chars().count()).sum()
        }
        FormattedTextLine::OrderedList(list) => {
            let prefix_chars = format!("{}. ", list.number.unwrap_or(1)).chars().count();
            prefix_chars
                + list
                    .indented_text
                    .text
                    .iter()
                    .map(|f| f.text.chars().count())
                    .sum::<usize>()
        }
        FormattedTextLine::UnorderedList(list) => {
            3 + list.text.iter().map(|f| f.text.chars().count()).sum::<usize>()
        }
        FormattedTextLine::CodeBlock(code) => code.code.chars().count(),
        FormattedTextLine::Table(table) => table
            .headers
            .iter()
            .chain(table.rows.iter().flatten())
            .flat_map(|cell| cell.iter())
            .map(|f| f.text.chars().count())
            .sum(),
        FormattedTextLine::Image(_)
        | FormattedTextLine::HorizontalRule
        | FormattedTextLine::LineBreak
        | FormattedTextLine::Embedded(_) => 0,
    }
}

fn block_char_count(block: &RichBlock) -> usize {
    let text = match block {
        RichBlock::Markdown(text) | RichBlock::Quote(text) => text,
    };
    text.lines.iter().map(formatted_line_char_count).sum()
}

fn segment_bloom(
    char_offset: &mut usize,
    seg_chars: usize,
    bloom: Option<GlyphBloomSchedule>,
) -> Option<GlyphBloomSchedule> {
    let start = *char_offset;
    let end = start + seg_chars;
    *char_offset = end;
    let schedule = bloom?;
    if seg_chars == 0 || end <= schedule.reveal_start_char {
        return None;
    }
    let local_start = schedule.reveal_start_char.saturating_sub(start);
    let skipped = start.saturating_sub(schedule.reveal_start_char);
    Some(GlyphBloomSchedule {
        reveal_start_char: local_start,
        elapsed_ms: (schedule.elapsed_ms - skipped as f32 * schedule.stagger_ms).max(0.0),
        stagger_ms: schedule.stagger_ms,
        duration_ms: schedule.duration_ms,
    })
}

impl Companion {
    fn work_formatted(
        &self,
        text: FormattedText,
        weight: Option<Weight>,
        alignment: TextAlignment,
        bloom: Option<GlyphBloomSchedule>,
    ) -> El {
        let p = self.p();
        let mut element = FormattedTextElement::new(
            text,
            BODY,
            self.font,
            self.mono_font,
            p.text,
            Default::default(),
        )
        .set_selectable(true)
        .with_line_height_ratio(1.5)
        .with_heading_to_font_size_multipliers(HeadingFontSizeMultipliers {
            h1: 1.4,
            h2: 1.25,
            h3: 1.18,
            h4: 1.1,
            h5: 1.0,
            h6: 1.0,
        })
        .with_hyperlink_font_color(p.blue)
        .with_inline_code_properties(Some(p.text), Some(p.surface))
        .with_alignment(alignment)
        .with_optical_dark_weight(p.is_dark)
        .with_glyph_bloom(bloom)
        .register_default_click_handlers_with_action_support(|link, ctx, _| {
            if let HyperlinkLens::Url(url) = link {
                if safe_link(url) {
                    ctx.dispatch_typed_action(Action::Work(WorkAction::OpenRemote(
                        url.to_string(),
                    )));
                }
            }
        });
        if let Some(weight) = weight {
            element = element.with_weight(p.optical_weight(weight));
        }
        element.finish()
    }

    fn work_markdown_table(
        &self,
        table: &FormattedTable,
        id: &str,
        char_offset: &mut usize,
        bloom: Option<GlyphBloomSchedule>,
    ) -> El {
        let p = self.p();
        let columns = table.headers.len().max(1);
        let key = format!("work-table-full-{id}");
        let expanded = self.amoo_expanded_tool_records.contains(&key);
        let row_limit = if expanded { usize::MAX } else { 12 };
        let mut rows = Vec::new();
        for (row_index, row) in std::iter::once(&table.headers)
            .chain(table.rows.iter().take(row_limit))
            .enumerate()
        {
            let cells = (0..columns)
                .map(|column| {
                    let content = row.get(column).cloned().unwrap_or_default();
                    let cell_chars: usize = content.iter().map(|f| f.text.chars().count()).sum();
                    let cell_bloom = segment_bloom(char_offset, cell_chars, bloom);
                    let alignment = match table.alignments.get(column) {
                        Some(TableAlignment::Center) => TextAlignment::Center,
                        Some(TableAlignment::Right) => TextAlignment::Right,
                        _ => TextAlignment::Left,
                    };
                    Expanded::new(
                        1.,
                        Container::new(self.work_formatted(
                            FormattedText::new([FormattedTextLine::Line(content)]),
                            (row_index == 0).then_some(Weight::Semibold),
                            alignment,
                            cell_bloom,
                        ))
                        .with_uniform_padding(S2)
                        .with_border(Border::right(1.).with_border_color(p.border))
                        .finish(),
                    )
                    .finish()
                })
                .collect::<Vec<_>>();
            rows.push(
                Container::new(
                    Flex::row()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .with_children(cells)
                        .finish(),
                )
                .with_background_color(if row_index == 0 || row_index % 2 == 0 {
                    p.surface
                } else {
                    p.bg
                })
                .with_border(Border::bottom(1.).with_border_color(p.border))
                .finish(),
            );
        }
        if table.rows.len() > 12 {
            if !expanded {
                let skipped_chars: usize = table
                    .rows
                    .iter()
                    .skip(row_limit)
                    .flatten()
                    .flat_map(|cell| cell.iter())
                    .map(|f| f.text.chars().count())
                    .sum();
                *char_offset += skipped_chars;
            }
            rows.push(self.button(
                format!("{key}-toggle"),
                if expanded {
                    "收起表格".into()
                } else {
                    format!("查看全部 {} 行", table.rows.len())
                },
                Action::AmooToggleToolRecords(key),
                Btn::Ghost,
            ));
        }
        Container::new(self.vstack(0., rows))
            .with_border(Border::all(1.).with_border_color(p.border))
            .finish()
    }

    fn work_markdown_lines(
        &self,
        text: &FormattedText,
        id: &str,
        char_offset: &mut usize,
        bloom: Option<GlyphBloomSchedule>,
    ) -> El {
        let p = self.p();
        let mut elements = Vec::new();
        let mut lines = Vec::new();
        let mut pending_chars = 0usize;
        let flush = |elements: &mut Vec<El>,
                     lines: &mut Vec<FormattedTextLine>,
                     pending_chars: &mut usize,
                     char_offset: &mut usize| {
            if !lines.is_empty() {
                let seg_bloom = segment_bloom(char_offset, *pending_chars, bloom);
                *pending_chars = 0;
                elements.push(self.work_formatted(
                    FormattedText::new(std::mem::take(lines)),
                    None,
                    TextAlignment::Left,
                    seg_bloom,
                ));
            }
        };
        for (index, line) in text.lines.iter().enumerate() {
            match line {
                FormattedTextLine::Table(table) => {
                    flush(&mut elements, &mut lines, &mut pending_chars, char_offset);
                    elements.push(self.work_markdown_table(
                        table,
                        &format!("{id}-{index}"),
                        char_offset,
                        bloom,
                    ));
                }
                FormattedTextLine::CodeBlock(code) => {
                    flush(&mut elements, &mut lines, &mut pending_chars, char_offset);
                    let body = code.code.clone();
                    let code_chars = body.chars().count();
                    let code_bloom = segment_bloom(char_offset, code_chars, bloom);
                    let line_count = body.lines().count();
                    let key = format!("work-code-expanded-{id}-{index}");
                    let collapsible = line_count > 8 || code_chars > 480;
                    let expanded = !collapsible || self.amoo_expanded_tool_records.contains(&key);
                    let mut contents = vec![self.between(
                        if collapsible {
                            self.button(
                                format!("{key}-toggle"),
                                format!(
                                    "{} 代码 · {line_count} 行",
                                    if expanded { "▾" } else { "›" }
                                ),
                                Action::AmooToggleToolRecords(key),
                                Btn::Ghost,
                            )
                        } else {
                            self.line(format!("代码 · {line_count} 行"), MICRO, p.text3)
                        },
                        self.work_action_icon(
                            format!("work-code-copy-{id}-{index}"),
                            "复制代码",
                            UiIcon::Copy,
                            Action::CopyText(body.clone(), "已复制代码".into()),
                        ),
                    )];
                    if expanded {
                        contents.push(
                            Text::new(body, self.mono_font, CAPTION)
                                .with_color(p.text)
                                .with_line_height_ratio(1.5)
                                .with_glyph_bloom(code_bloom)
                                .finish(),
                        );
                    }
                    elements.push(
                        Container::new(self.vstack(S2, contents))
                            .with_uniform_padding(S3)
                            .with_background_color(p.surface)
                            .with_border(Border::all(1.).with_border_color(p.border))
                            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                            .finish(),
                    );
                }
                FormattedTextLine::HorizontalRule => {
                    flush(&mut elements, &mut lines, &mut pending_chars, char_offset);
                    elements.push(self.divider());
                }
                FormattedTextLine::Image(image) => {
                    flush(&mut elements, &mut lines, &mut pending_chars, char_offset);
                    let label = if image.alt_text.is_empty() {
                        "图片"
                    } else {
                        &image.alt_text
                    };
                    elements.push(self.amoo_attachment_card(
                        label,
                        "image",
                        "",
                        if safe_link(&image.source) {
                            &image.source
                        } else {
                            ""
                        },
                        None,
                    ));
                }
                other => {
                    pending_chars += formatted_line_char_count(other);
                    lines.push(other.clone());
                }
            }
        }
        flush(&mut elements, &mut lines, &mut pending_chars, char_offset);
        self.vstack(S3, elements)
    }

    pub(in super::super) fn work_rich_text(&self, body: &str, id: &str) -> El {
        let bloom = self.reply_motion.take_render_bloom(id);
        self.work_rich_text_bloom(body, id, bloom)
    }

    pub(in super::super) fn work_rich_text_bloom(
        &self,
        body: &str,
        id: &str,
        bloom: Option<GlyphBloomSchedule>,
    ) -> El {
        let p = self.p();
        let blocks = RICH_TEXT_CACHE.with(|cache| cache.borrow_mut().get(body));
        let adjusted_bloom = bloom.map(|mut schedule| {
            let raw_total = body.chars().count();
            let rendered_total: usize = blocks.iter().map(block_char_count).sum();
            if rendered_total < raw_total {
                let frontier = raw_total.saturating_sub(schedule.reveal_start_char);
                schedule.reveal_start_char = rendered_total.saturating_sub(frontier);
            }
            schedule
        });
        let mut char_offset = 0usize;
        self.vstack(
            S3,
            blocks
                .iter()
                .enumerate()
                .map(|(index, block)| match block {
                    RichBlock::Markdown(text) => self.work_markdown_lines(
                        text,
                        &format!("{id}-{index}"),
                        &mut char_offset,
                        adjusted_bloom,
                    ),
                    RichBlock::Quote(text) => Container::new(self.work_markdown_lines(
                        text,
                        &format!("{id}-quote-{index}"),
                        &mut char_offset,
                        adjusted_bloom,
                    ))
                    .with_uniform_padding(S3)
                    .with_background_color(p.surface)
                    .with_border(Border::left(3.).with_border_color(p.border_strong))
                    .finish(),
                })
                .collect(),
        )
    }

    pub(in super::super) fn render_rich_message_bloom(
        &self,
        body: &str,
        id: &str,
        bloom: Option<GlyphBloomSchedule>,
    ) -> El {
        self.work_rich_text_bloom(body, id, bloom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inline_images_match_rendering_and_ignore_code_or_unsafe_sources() {
        let images=markdown_images("![首页](https://example.test/a.png?sig=abc)\n\n> ![引用](https://example.test/b)\n\n```md\n![示例](https://example.test/not-loaded)\n```\n\n![重复](https://example.test/a.png?sig=abc)\n\n![禁止](file:///tmp/private)");
        assert_eq!(images.len(), 2);
        assert_eq!(images[0].name, "首页");
        assert!(images[0].url.ends_with("?sig=abc"));
        assert_eq!(images[1].name, "引用");
    }
    #[test]
    fn parsed_text_cache_is_bounded_and_separates_edits() {
        let mut cache = RichTextCache::default();
        cache.get("**中文**");
        cache.get("**中文**");
        assert_eq!(cache.entries.len(), 1);
        cache.get("**修改**");
        assert_eq!(cache.entries.len(), 2);
        for i in 0..100 {
            cache.get(&format!("记录 {i}"));
        }
        assert_eq!(cache.entries.len(), 64);
        assert!(cache.bytes <= 2 * 1024 * 1024);
        cache.get(&"x".repeat(256 * 1024 + 1));
        assert_eq!(cache.entries.len(), 64);
    }

    #[test]
    fn markdown_keeps_chinese_bold_lists_code_and_tables() {
        let blocks = parse_blocks("## 分析结果\n\n**总计**：`42`\n\n1. 检查资料\n2. 输出报告\n\n| 指标 | 值 |\n| --- | ---: |\n| 合计 | **42** |\n\n```csv\n指标,值\n合计,42\n```\n");
        let RichBlock::Markdown(text) = &blocks[0] else {
            panic!()
        };
        assert!(text
            .lines
            .iter()
            .any(|l| matches!(l, FormattedTextLine::Heading(_))));
        assert!(text
            .lines
            .iter()
            .any(|l| matches!(l, FormattedTextLine::OrderedList(_))));
        assert!(text.lines.iter().any(
            |l| matches!(l, FormattedTextLine::Table(t) if t.headers.len()==2 && t.rows.len()==1)
        ));
        assert!(text
            .lines
            .iter()
            .any(|l| matches!(l, FormattedTextLine::CodeBlock(c) if c.code.contains("指标,值"))));
        let serialized = format!("{text:?}");
        assert!(serialized.contains("Bold"));
        assert!(serialized.contains("InlineCode"));
    }
    #[test]
    fn quotes_are_separate_but_code_literals_are_preserved() {
        let blocks = parse_blocks("正文\n> 引用 **结论**\n> 第二行\n\n```text\n> 这是代码\n```\n");
        assert_eq!(blocks.len(), 3);
        assert!(matches!(&blocks[1], RichBlock::Quote(_)));
        assert!(
            matches!(&blocks[2], RichBlock::Markdown(text) if text.lines.iter().any(|line| matches!(line, FormattedTextLine::CodeBlock(code) if code.code.contains("> 这是代码"))))
        );
    }
    #[test]
    fn links_only_open_explicit_web_destinations() {
        for url in [
            "javascript:alert(1)",
            "file:///etc/passwd",
            "https://",
            "https://example.com/\ncommand",
            "data:text/html,test",
        ] {
            assert!(!safe_link(url), "{url}");
        }
        assert!(safe_link("https://example.com/中文?q=test#result"));
        let mut line =
            FormattedTextLine::Line(vec![markdown_parser::FormattedTextFragment::hyperlink(
                "unsafe",
                "javascript:alert(1)",
            )]);
        sanitize_links(&mut line);
        assert!(line.hyperlinks(false).is_empty());
        assert!(line.raw_text().contains("unsafe"));
    }
    #[test]
    fn segment_bloom_skips_settled_blocks_and_offsets_trailing_frontier() {
        let schedule = Some(GlyphBloomSchedule {
            reveal_start_char: 8,
            elapsed_ms: 48.0,
            stagger_ms: 16.0,
            duration_ms: 90.0,
        });
        let mut offset = 0;
        assert!(segment_bloom(&mut offset, 6, schedule).is_none());
        let first_active = segment_bloom(&mut offset, 4, schedule).unwrap();
        assert_eq!(first_active.reveal_start_char, 2);
        assert_eq!(first_active.elapsed_ms, 48.0);
        let second_active = segment_bloom(&mut offset, 5, schedule).unwrap();
        assert_eq!(second_active.reveal_start_char, 0);
        assert!((second_active.elapsed_ms - 16.0).abs() < 1e-3);
    }
}
