//! The main module that contains the [`Engine`] struct and its related types.
use std::sync::Arc;

use kbnf_syntax::simplified_grammar::SimplifiedGrammar;
use num::Bounded;
use serde::{Deserialize, Serialize};

use crate::{
    config::Config, engine_base::EngineBase, engine_like::EngineLike, grammar::Grammar, utils,
    vocabulary::Vocabulary, zero::Zero,
};

/// The specific config of the [`Engine`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct EngineConfig {
    /// Whether the cache is enabled. Caching speeds up the engine eventually if any of the following conditions are met:
    /// 1. The grammar is "simple". What exactly constitutes a simple grammar is not well defined at the moment but
    /// all regular grammars should be simple.
    /// 2. The grammar is reused multiple times for inputs of similar lengths.
    /// It is enabled by default.
    pub cache_enabled: bool,
    /// Whether the compaction is enabled. Compaction reduces the memory usage of the engine and
    /// speeds up the engine in most cases. In particular, cache usually requires compaction to be effective.
    /// It is enabled by default.
    pub compaction_enabled: bool,
}
#[derive(Debug, Clone)]
/// An enum that represents the common type combinations of [`EngineBase`].
pub(crate) enum EngineUnion {
    /// Typical simple grammar with complex dfa without any repetition
    U8U0U8U8U8U32(EngineBase<u8, Zero, u8, u8, u8, u32>),
    /// Typical simple grammar with simple dfa without any repetition
    U8U0U8U16U16U16(EngineBase<u8, Zero, u8, u16, u16, u16>),
    /// Complex grammar with complex dfa without any repetition
    U16U0U16U32U32U32(EngineBase<u16, Zero, u16, u32, u32, u32>),
    /// Typical simple grammar with complex dfa
    U8U8U8U8U8U32(EngineBase<u8, u8, u8, u8, u8, u32>),
    /// Typical simple grammar with simple dfa
    U8U8U8U16U16U16(EngineBase<u8, u8, u8, u16, u16, u16>),
    /// Complex grammar with complex dfa
    U16U8U16U32U32U32(EngineBase<u16, u8, u16, u32, u32, u32>),
    /// Typical simple grammar with simple dfa and unusually large repetitions
    U8U16U8U8U8U32(EngineBase<u8, u16, u8, u8, u8, u32>),
    /// Complex grammar with complex dfa and unusually large repetitions
    U16U16U16U32U32U32(EngineBase<u16, u16, u16, u32, u32, u32>),
}
#[derive(Debug, Clone)]
/// The main struct that wraps the [`EngineBase`] so the user do not have to specify the generic type every time for common cases.
pub struct Engine {
    union: EngineUnion,
}
#[derive(Debug, thiserror::Error)]
/// Represents the error type for the [`Engine`] creation.
pub enum CreateEngineError {
    #[error("{0}")] // inherits the error message from the wrapped EngineBaseError
    /// A wrapper for the [`CreateEngineBaseError`](crate::engine_base::CreateEngineBaseError) error type.
    EngineBaseError(#[from] crate::engine_base::CreateEngineBaseError),
    #[error("{0}")] // inherits the error message from the wrapped GrammarError
    /// A wrapper for the [`CreateGrammarError`](crate::grammar::CreateGrammarError) error type.
    GrammarError(#[from] crate::grammar::CreateGrammarError),
    #[error("The grammar after simplification is empty.
    This usually means that the grammar only contains empty terminals and/or self recursions like A::=A;")]
    /// The grammar is empty.
    EmptyGrammarError,
    #[error("The grammar and/or config's value range is not supported by the Engine.\n
    This usually means that the grammar has more than 65536 nonterminals,
    at least one nonterminal has more than 65536 alternations or repetitions, and/or the expected output length is more than 2^32.")]
    /// The grammar and/or config's value range is not supported by the Engine.
    InvalidInputError,
}

impl Engine {
    /// Create a new [`Engine`] from an EBNF grammar string and a [`Vocabulary`].
    ///
    /// # Arguments
    ///
    /// * `kbnf_syntax_grammar_str` - The EBNF grammar string.
    ///
    /// * `vocabulary` - The [`Vocabulary`] object.
    ///
    /// # Returns
    ///
    /// * [`Engine`] - The new [`Engine`] object.
    ///
    /// # Errors
    ///
    /// Returns an [`CreateEngineError`] when the grammar is empty or the grammar and/or config's value range is not supported by the Engine.
    pub fn new(
        kbnf_syntax_grammar_str: &str,
        vocabulary: Vocabulary,
    ) -> Result<Self, CreateEngineError> {
        let config = Config::default();
        Self::with_config(kbnf_syntax_grammar_str, vocabulary, config)
    }

    fn check_id_length(grammar: &SimplifiedGrammar, value: usize) -> bool {
        grammar.interned_strings.terminals.len() <= value
            && grammar.interned_strings.nonterminals.len() <= value
            && grammar.interned_strings.excepteds.len() <= value
    }
    /// Create a new [`Engine`] from an EBNF grammar string, a [`Vocabulary`], and a [`Config`].
    ///
    /// # Arguments
    ///
    /// * `kbnf_syntax_grammar_str` - The EBNF grammar string.
    /// * `vocabulary` - The [`Vocabulary`] object.
    /// * `config` - The [`Config`] object.
    ///
    /// # Returns
    ///
    /// * [`Engine`] - The new [`Engine`] object.
    ///
    /// # Errors
    ///
    /// Returns an [`CreateEngineError`] when the grammar is empty or the grammar and/or config's value range is not supported by the Engine.
    pub fn with_config(
        kbnf_syntax_grammar_str: &str,
        vocabulary: Vocabulary,
        config: Config,
    ) -> Result<Self, CreateEngineError> {
        let tsp = config.expected_output_length;
        let internal_config = config.internal_config();
        let grammar =
            utils::construct_kbnf_syntax_grammar(kbnf_syntax_grammar_str, internal_config.clone())?;
        if grammar.is_empty() {
            return Err(CreateEngineError::EmptyGrammarError);
        }
        let max_r = utils::find_max_repetition_from_kbnf_syntax_grammar(&grammar);
        let td = utils::find_max_dotted_position_from_kbnf_syntax_grammar(&grammar);
        let tp = utils::find_max_production_id_from_kbnf_syntax_grammar(&grammar);
        let ts = utils::find_max_state_id_from_kbnf_syntax_grammar(&grammar);
        let engine = if Self::check_id_length(&grammar, u8::MAX.into())
            && max_r <= Zero::max_value().into()
            && td <= u8::MAX.into()
            && tp <= u8::MAX.into()
            && tsp <= u8::MAX.into()
            && ts <= u32::MAX as usize
        {
            let grammar: Grammar<u8, Zero> = Grammar::new(grammar)?;
            let grammar = Arc::new(grammar);
            let vocabulary = Arc::new(vocabulary);
            EngineUnion::U8U0U8U8U8U32(EngineBase::new(
                vocabulary,
                grammar,
                internal_config.engine_config,
            )?)
        } else if Self::check_id_length(&grammar, u8::MAX.into())
            && max_r <= Zero::max_value().into()
            && td <= u8::MAX.into()
            && tp <= u16::MAX.into()
            && tsp <= u16::MAX.into()
            && ts <= u16::MAX as usize
        {
            let grammar: Grammar<u8, Zero> = Grammar::new(grammar)?;
            let grammar = Arc::new(grammar);
            let vocabulary = Arc::new(vocabulary);
            EngineUnion::U8U0U8U16U16U16(EngineBase::new(
                vocabulary,
                grammar,
                internal_config.engine_config,
            )?)
        } else if Self::check_id_length(&grammar, u8::MAX.into())
            && max_r <= u16::max_value().into()
            && td <= u8::MAX.into()
            && tp <= u8::MAX.into()
            && tsp <= u8::MAX.into()
            && ts <= u32::MAX as usize
        {
            let grammar: Grammar<u8, u16> = Grammar::new(grammar)?;
            let grammar = Arc::new(grammar);
            let vocabulary = Arc::new(vocabulary);
            EngineUnion::U8U16U8U8U8U32(EngineBase::new(
                vocabulary,
                grammar,
                internal_config.engine_config,
            )?)
        } else if Self::check_id_length(&grammar, u16::MAX.into())
            && max_r <= Zero::max_value().into()
            && td <= u16::MAX.into()
            && tp <= u32::MAX as usize
            && tsp <= u32::MAX as usize
            && ts <= u32::MAX as usize
        {
            let grammar: Grammar<u16, Zero> = Grammar::new(grammar)?;
            let grammar = Arc::new(grammar);
            let vocabulary = Arc::new(vocabulary);
            EngineUnion::U16U0U16U32U32U32(EngineBase::new(
                vocabulary,
                grammar,
                internal_config.engine_config,
            )?)
        } else if Self::check_id_length(&grammar, u8::MAX.into())
            && max_r <= u8::max_value().into()
            && td <= u8::MAX.into()
            && tp <= u8::MAX.into()
            && tsp <= u8::MAX.into()
            && ts <= u32::MAX as usize
        {
            let grammar: Grammar<u8, u8> = Grammar::new(grammar)?;
            let grammar = Arc::new(grammar);
            let vocabulary = Arc::new(vocabulary);
            EngineUnion::U8U8U8U8U8U32(EngineBase::new(
                vocabulary,
                grammar,
                internal_config.engine_config,
            )?)
        } else if Self::check_id_length(&grammar, u8::MAX.into())
            && max_r <= u8::max_value().into()
            && td <= u8::MAX.into()
            && tp <= u16::MAX.into()
            && tsp <= u16::MAX.into()
            && ts <= u16::MAX as usize
        {
            let grammar: Grammar<u8, u8> = Grammar::new(grammar)?;
            let grammar = Arc::new(grammar);
            let vocabulary = Arc::new(vocabulary);
            EngineUnion::U8U8U8U16U16U16(EngineBase::new(
                vocabulary,
                grammar,
                internal_config.engine_config,
            )?)
        } else if Self::check_id_length(&grammar, u16::MAX.into())
            && max_r <= u8::max_value().into()
            && td <= u16::MAX.into()
            && tp <= u32::MAX as usize
            && tsp <= u32::MAX as usize
            && ts <= u32::MAX as usize
        {
            let grammar: Grammar<u16, u8> = Grammar::new(grammar)?;
            let grammar = Arc::new(grammar);
            let vocabulary = Arc::new(vocabulary);
            EngineUnion::U16U8U16U32U32U32(EngineBase::new(
                vocabulary,
                grammar,
                internal_config.engine_config,
            )?)
        } else if Self::check_id_length(&grammar, u16::MAX.into())
            && max_r <= u16::max_value().into()
            && td <= u16::MAX.into()
            && tp <= u32::MAX as usize
            && tsp <= u32::MAX as usize
            && ts <= u32::MAX as usize
        {
            let grammar: Grammar<u16, u16> = Grammar::new(grammar)?;
            let grammar = Arc::new(grammar);
            let vocabulary = Arc::new(vocabulary);
            EngineUnion::U16U16U16U32U32U32(EngineBase::new(
                vocabulary,
                grammar,
                internal_config.engine_config,
            )?)
        } else {
            return Err(CreateEngineError::InvalidInputError);
        };
        Ok(Self { union: engine })
    }
}

macro_rules! match_engine_union {
    ($s:expr, $e:path$(,$p:ident)*) => {
        match $s {
            EngineUnion::U8U0U8U8U8U32(engine) => $e(engine, $($p,)*),
            EngineUnion::U8U0U8U16U16U16(engine) => $e(engine, $($p,)*),
            EngineUnion::U16U0U16U32U32U32(engine) => $e(engine, $($p,)*),
            EngineUnion::U8U8U8U8U8U32(engine) => $e(engine, $($p,)*),
            EngineUnion::U8U8U8U16U16U16(engine) => $e(engine, $($p,)*),
            EngineUnion::U16U8U16U32U32U32(engine) => $e(engine, $($p,)*),
            EngineUnion::U8U16U8U8U8U32(engine) => $e(engine, $($p,)*),
            EngineUnion::U16U16U16U32U32U32(engine) => $e(engine, $($p,)*),
        }
    }
}

impl EngineLike for Engine {
    fn try_accept_new_token(
        &mut self,
        token_id: u32,
    ) -> Result<crate::engine_like::AcceptTokenResult, crate::engine_like::AcceptTokenError> {
        match_engine_union!(&mut self.union, EngineLike::try_accept_new_token, token_id)
    }

    fn compute_allowed_token_ids(&mut self) {
        match_engine_union!(&mut self.union, EngineLike::compute_allowed_token_ids)
    }

    fn mask_logits(&self, logits: &mut [f32]) -> Result<(), crate::engine_like::MaskLogitsError> {
        match_engine_union!(&self.union, EngineLike::mask_logits, logits)
    }

    fn update_logits(
        &mut self,
        token_id: u32,
        logits: &mut [f32],
    ) -> Result<crate::engine_like::AcceptTokenResult, crate::engine_like::UpdateLogitsError> {
        match_engine_union!(&mut self.union, EngineLike::update_logits, token_id, logits)
    }

    fn allowed_token_ids_from_last_computation(&self) -> &fixedbitset_stack::FixedBitSet {
        match_engine_union!(
            &self.union,
            EngineLike::allowed_token_ids_from_last_computation
        )
    }

    fn is_finished(&self) -> bool {
        match_engine_union!(&self.union, EngineLike::is_finished)
    }

    fn reset(&mut self) {
        match_engine_union!(&mut self.union, EngineLike::reset)
    }

    fn into_boxed_engine(self) -> Box<dyn EngineLike> {
        match_engine_union!(self.union, EngineLike::into_boxed_engine)
    }
    fn vocab(&self) -> Arc<Vocabulary> {
        match_engine_union!(&self.union, EngineLike::vocab)
    }
}
