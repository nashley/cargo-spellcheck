//! A nlp based rule checker base on `nlprule`
//!
//! Does check grammar, and is supposed to only check for grammar.
//! Sentence splitting is done in hand-waving way. To be improved.

use super::{Checker, Detector, Documentation, Suggestion, SuggestionSet};
use crate::{CheckableChunk, ContentOrigin};

use log::{debug, trace};
use rayon::prelude::*;

use anyhow::Result;

use nlprule::types::Suggestion as NlpFix;
use nlprule::{Rules, Tokenizer};

static TOKENIZER_BYTES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/tokenizer.bin"));
static RULES_BYTES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/rules.bin"));

lazy_static::lazy_static! {
    static ref TOKENIZER: Tokenizer = Tokenizer::new_from(&mut &*TOKENIZER_BYTES).unwrap();
    static ref RULES: Rules = Rules::new_from(&mut &*RULES_BYTES).unwrap();
}

pub(crate) struct NlpRulesChecker;

impl Checker for NlpRulesChecker {
    type Config = ();
    fn check<'a, 's>(docu: &'a Documentation, _config: &Self::Config) -> Result<SuggestionSet<'s>>
    where
        'a: 's,
    {
        // dbg!(RULES.rules());
        let mut suggestions = docu
            .par_iter()
            .try_fold::<SuggestionSet, Result<_>, _, _>(
                || SuggestionSet::new(),
                move |mut acc, (origin, chunks)| {
                    debug!("Processing {}", origin.as_path().display());

                    for chunk in chunks {
                        acc.extend(
                            origin.clone(),
                            check_sentence(origin.clone(), chunk, &TOKENIZER, &RULES),
                        );
                    }
                    Ok(acc)
                },
            )
            .try_reduce(
                || SuggestionSet::new(),
                |mut a, b| {
                    a.join(b);
                    Ok(a)
                },
            )?;

        suggestions.sort();
        Ok(suggestions)
    }
}

/// Check one segmented sentence
fn check_sentence<'a>(
    origin: ContentOrigin,
    chunk: &'a CheckableChunk,
    tokenizer: &Tokenizer,
    rules: &Rules,
) -> Vec<Suggestion<'a>> {
    let plain = chunk.erase_cmark();
    trace!("{:?}", &plain);
    let txt = plain.as_str();

    let mut acc = Vec::with_capacity(32);

    let mut history = Vec::with_capacity(8);
    'sentence: for sentence in txt
        .split(|c: char| {
            let previous = history.pop();
            history.push(c);
            // FIXME use a proper segmenter
            match c {
                '.' | '!' | '?' | ';' => true,
                '\n' if previous == Some('\n') => true, // FIXME other line endings
                _ => false,
            }
        })
        .filter(|sentence| !sentence.is_empty())
    {
        let nlpfixes = rules.suggest(sentence, tokenizer);
        if nlpfixes.is_empty() {
            continue 'sentence;
        }

        'nlp: for NlpFix {
            message,
            start,
            end,
            replacements,
            ..
        } in nlpfixes
        {
            if start > end {
                continue 'nlp;
            }
            let range = start..(end + 1);
            acc.extend(
                plain
                    .find_spans(range)
                    .into_iter()
                    .map(|(range, span)| Suggestion {
                        detector: Detector::NlpRules,
                        range,
                        span,
                        origin: origin.clone(),
                        replacements: replacements.iter().map(|x| x.clone()).collect(),
                        chunk,
                        description: Some(message.clone()),
                    }),
            );
        }
    }

    acc
}