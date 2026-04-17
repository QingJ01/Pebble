use tantivy::schema::{
    DateOptions, Field, IndexRecordOption, Schema, SchemaBuilder, TextFieldIndexing, TextOptions,
    INDEXED, STORED, STRING,
};
use tantivy::tokenizer::{NgramTokenizer, TextAnalyzer, Token, TokenStream, Tokenizer};
use tantivy::{DateTimePrecision, Index};

const NGRAM_TOKENIZER: &str = "ngram3";
pub(crate) const BODY_TOKENIZER: &str = "body_cjk";

/// Tokenizer that uses standard word splitting for Latin text and emits
/// individual characters for CJK scripts. Avoids n-gram bloat on large
/// bodies while keeping CJK searchable.
#[derive(Clone)]
struct CjkAwareTokenizer;

struct CjkAwareTokenStream {
    tokens: Vec<Token>,
    index: usize,
}

impl Tokenizer for CjkAwareTokenizer {
    type TokenStream<'a> = CjkAwareTokenStream;

    fn token_stream<'a>(&'a mut self, text: &'a str) -> Self::TokenStream<'a> {
        let mut tokens = Vec::new();
        let mut pos = 0usize;
        let mut word_start: Option<usize> = None;
        let mut position: usize = 0;

        for (byte_offset, ch) in text.char_indices() {
            let is_cjk = is_cjk_char(ch);
            if is_cjk {
                // Flush any pending Latin word
                if let Some(start) = word_start.take() {
                    let word = &text[start..byte_offset];
                    let lower = word.to_lowercase();
                    if !lower.is_empty() {
                        tokens.push(Token {
                            offset_from: start,
                            offset_to: byte_offset,
                            position,
                            position_length: 1,
                            text: lower,
                        });
                        position += 1;
                    }
                }
                // Emit CJK character as its own token
                let end = byte_offset + ch.len_utf8();
                tokens.push(Token {
                    offset_from: byte_offset,
                    offset_to: end,
                    position,
                    position_length: 1,
                    text: ch.to_string(),
                });
                position += 1;
            } else if ch.is_alphanumeric() {
                if word_start.is_none() {
                    word_start = Some(byte_offset);
                }
            } else {
                // Separator — flush pending word
                if let Some(start) = word_start.take() {
                    let word = &text[start..byte_offset];
                    let lower = word.to_lowercase();
                    if !lower.is_empty() {
                        tokens.push(Token {
                            offset_from: start,
                            offset_to: byte_offset,
                            position,
                            position_length: 1,
                            text: lower,
                        });
                        position += 1;
                    }
                }
            }
            pos = byte_offset + ch.len_utf8();
        }
        // Flush trailing word
        if let Some(start) = word_start {
            let word = &text[start..pos];
            let lower = word.to_lowercase();
            if !lower.is_empty() {
                tokens.push(Token {
                    offset_from: start,
                    offset_to: pos,
                    position,
                    position_length: 1,
                    text: lower,
                });
            }
        }

        CjkAwareTokenStream { tokens, index: 0 }
    }
}

impl TokenStream for CjkAwareTokenStream {
    fn advance(&mut self) -> bool {
        if self.index < self.tokens.len() {
            self.index += 1;
            true
        } else {
            false
        }
    }

    fn token(&self) -> &Token {
        &self.tokens[self.index - 1]
    }

    fn token_mut(&mut self) -> &mut Token {
        &mut self.tokens[self.index - 1]
    }
}

fn is_cjk_char(c: char) -> bool {
    matches!(c,
        '\u{4E00}'..='\u{9FFF}'   // CJK Unified Ideographs
        | '\u{3400}'..='\u{4DBF}' // CJK Extension A
        | '\u{F900}'..='\u{FAFF}' // CJK Compatibility Ideographs
        | '\u{3040}'..='\u{309F}' // Hiragana
        | '\u{30A0}'..='\u{30FF}' // Katakana
        | '\u{AC00}'..='\u{D7AF}' // Hangul Syllables
    )
}

pub struct SearchSchema {
    pub schema: Schema,
    pub message_id: Field,
    pub subject: Field,
    pub body_text: Field,
    pub from_address: Field,
    pub from_name: Field,
    pub to_addresses: Field,
    pub date: Field,
    pub folder_id: Field,
    pub account_id: Field,
    pub has_attachment: Field,
}

pub fn build_schema() -> SearchSchema {
    let mut builder: SchemaBuilder = Schema::builder();

    let message_id = builder.add_text_field("message_id", STRING | STORED);

    // N-gram tokenizer for short fields where substring matching matters
    let ngram_stored = TextOptions::default()
        .set_indexing_options(
            TextFieldIndexing::default()
                .set_tokenizer(NGRAM_TOKENIZER)
                .set_index_option(IndexRecordOption::WithFreqsAndPositions),
        )
        .set_stored();

    let ngram_only = TextOptions::default().set_indexing_options(
        TextFieldIndexing::default()
            .set_tokenizer(NGRAM_TOKENIZER)
            .set_index_option(IndexRecordOption::WithFreqsAndPositions),
    );

    // CJK-aware tokenizer for body_text — standard word splitting for Latin,
    // character-level for CJK. Avoids n-gram bloat on large bodies.
    let body_stored = TextOptions::default()
        .set_indexing_options(
            TextFieldIndexing::default()
                .set_tokenizer(BODY_TOKENIZER)
                .set_index_option(IndexRecordOption::WithFreqsAndPositions),
        )
        .set_stored();

    let subject = builder.add_text_field("subject", ngram_stored.clone());
    let body_text = builder.add_text_field("body_text", body_stored);
    let from_address = builder.add_text_field("from_address", ngram_stored.clone());
    let from_name = builder.add_text_field("from_name", ngram_stored);
    let to_addresses = builder.add_text_field("to_addresses", ngram_only);

    let date_options = DateOptions::from(INDEXED | STORED)
        .set_precision(DateTimePrecision::Seconds);
    let date = builder.add_date_field("date", date_options);

    let folder_id = builder.add_text_field("folder_id", STRING);
    let account_id = builder.add_text_field("account_id", STRING);
    let has_attachment = builder.add_text_field("has_attachment", STRING);

    let schema = builder.build();

    SearchSchema {
        schema,
        message_id,
        subject,
        body_text,
        from_address,
        from_name,
        to_addresses,
        date,
        folder_id,
        account_id,
        has_attachment,
    }
}

/// Register custom tokenizers on the index. Must be called after index creation.
pub fn register_tokenizers(index: &Index) {
    let ngram = TextAnalyzer::builder(NgramTokenizer::new(2, 3, false).unwrap())
        .build();
    index.tokenizers().register(NGRAM_TOKENIZER, ngram);

    let body = TextAnalyzer::builder(CjkAwareTokenizer).build();
    index.tokenizers().register(BODY_TOKENIZER, body);
}
