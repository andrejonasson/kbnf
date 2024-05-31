use ahash::{AHashMap, AHashSet};
use ebnf::regex::FiniteStateAutomaton;
use fixedbitset::FixedBitSet;
use jaggedarray::jagged_array::JaggedArray;
use jaggedarray::jagged_array::JaggedArrayViewTrait;
use num::Bounded;
use num::CheckedSub;
use num::{
    cast::AsPrimitive,
    traits::{ConstOne, ConstZero, NumAssign, NumOps},
    Num,
};
use regex_automata::dfa::Automaton;
use regex_automata::hybrid::dfa::Cache;
use regex_automata::hybrid::LazyStateID;
use regex_automata::util::primitives::StateID;
use std::sync::Arc;

use crate::grammar::ExceptedID;
use crate::grammar::RegexID;
use crate::grammar::INVALID_REPETITION;
use crate::utils;
use crate::utils::ByteSet;
use crate::{
    grammar::{Grammar, LNFNode, NonterminalID},
    vocabulary::Vocabulary,
};
type EarleySets<TN, TD, TP, TSP, TS> = JaggedArray<EarleyItem<TN, TD, TP, TSP, TS>, Vec<usize>, 2>;
const USIZE_WIDTH: usize = std::mem::size_of::<usize>();
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct EarleyItem<TN, TD, TP, TSP, TS>
where
    TN: Num + AsPrimitive<usize> + ConstOne + ConstZero,
    TD: Num + AsPrimitive<usize> + ConstOne + ConstZero,
    TP: Num + AsPrimitive<usize> + ConstOne + ConstZero,
    TSP: Num + AsPrimitive<usize> + ConstOne + ConstZero,
    usize: num::traits::AsPrimitive<TN>
        + num::traits::AsPrimitive<TD>
        + num::traits::AsPrimitive<TP>
        + num::traits::AsPrimitive<TSP>,
{
    pub nonterminal_id: NonterminalID<TN>,
    pub dot_position: TD,
    pub production_index: TP,
    pub start_position: TSP,
    pub state_id: TS,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ToBeCompletedItem<TN, TSP>
where
    TN: Num + AsPrimitive<usize> + ConstOne + ConstZero + Eq + std::hash::Hash + PartialEq,
    TSP: Num + AsPrimitive<usize> + ConstOne + ConstZero + Eq + std::hash::Hash + PartialEq,
{
    nonterminal_id: NonterminalID<TN>,
    start_position: TSP,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Dotted<TN, TSP>
where
    TN: Num + AsPrimitive<usize> + ConstOne + ConstZero + Eq + std::hash::Hash + PartialEq,
    TSP: Num + AsPrimitive<usize> + ConstOne + ConstZero + Eq + std::hash::Hash + PartialEq,
{
    postdot_nonterminal_id: NonterminalID<TN>,
    column: TSP,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum PostDotItems<TN, TD, TP, TSP, TS>
where
    TN: Num + AsPrimitive<usize> + ConstOne + ConstZero,
    TD: Num + AsPrimitive<usize> + ConstOne + ConstZero,
    TP: Num + AsPrimitive<usize> + ConstOne + ConstZero,
    TSP: Num + AsPrimitive<usize> + ConstOne + ConstZero,
    usize: num::traits::AsPrimitive<TN>
        + num::traits::AsPrimitive<TD>
        + num::traits::AsPrimitive<TP>
        + num::traits::AsPrimitive<TSP>,
{
    LeoEligible(EarleyItem<TN, TD, TP, TSP, TS>),
    NormalItems(Vec<EarleyItem<TN, TD, TP, TSP, TS>>),
}

#[derive(Debug, Clone)]
pub struct EngineConfig {
    pub cache_enabled: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum EngineError {
    #[error(
        "Terminal length {0} exceeds {1}, the maximum terminal length allowed by current size of StateID(TS).
     Consider reducing terminal length or use larger StateID(TS)."
    )]
    TerminalTooLong(usize, usize),
    #[error(
        "Regex length {0} exceeds {1}, the maximum regex length allowed by current size of StateID(TS).
     Consider reducing regex states or use larger StateID(TS)."
    )]
    RegexTooLarge(usize, usize),
    #[error(
        "Except! length {0} exceeds {1}, the maximum excepted length allowed by current size of StateID(TS).
     Consider reducing excepted terminals, use larger StateID(TS) or less repetition."
    )]
    ExceptedTooLarge(usize, usize),
    #[error(
        "Repetition in regex {0} exceeds {1}, the maximum repetition allowed by current size of StateID(TS).
     Consider reducing repetition or use larger StateID(TS)."
    )]
    RepetitionInExceptedTooLarge(usize, usize),
}
#[allow(clippy::type_complexity)]
#[derive(Debug, Clone)]
pub struct EngineBase<TI, TE, TD, TP, TSP, TS>
where
    TI: Num + AsPrimitive<usize> + ConstOne + ConstZero + Eq + std::hash::Hash + PartialEq,
    TE: crate::non_zero::ConstOne + Eq + std::hash::Hash + PartialEq + AsPrimitive<usize> + Bounded,
    TD: Num + AsPrimitive<usize> + ConstOne + ConstZero + Eq + std::hash::Hash + PartialEq,
    TP: Num + AsPrimitive<usize> + ConstOne + ConstZero + Eq + std::hash::Hash + PartialEq,
    TSP: Num + AsPrimitive<usize> + ConstOne + ConstZero + Eq + std::hash::Hash + PartialEq,
    TS: Num + AsPrimitive<usize> + ConstOne + ConstZero + Eq + std::hash::Hash + PartialEq,
    usize: num::traits::AsPrimitive<TI>
        + num::traits::AsPrimitive<TD>
        + num::traits::AsPrimitive<TP>
        + num::traits::AsPrimitive<TSP>,
{
    vocabulary: Arc<Vocabulary>,
    grammar: Arc<Grammar<TI, TE>>,
    allowed_first_bytes: ByteSet,
    allowed_token_ids: FixedBitSet,
    earley_sets: EarleySets<TI, TD, TP, TSP, TS>,
    cache: AHashMap<EarleySets<TI, TD, TP, TSP, TS>, FixedBitSet>,
    regex_id_to_cache: AHashMap<RegexID<TI>, Cache>,
    excepted_id_to_cache: AHashMap<ExceptedID<TI>, Cache>,
    to_be_completed_items: AHashSet<ToBeCompletedItem<TI, TSP>>,
    to_be_completed_items_buffer: AHashSet<ToBeCompletedItem<TI, TSP>>,
    // Maybe a smallvec will be better. Profiling is needed to make a decision.
    // I feel like copying the item is better than add a reference to the item since the item is relatively small(<=16 bytes)
    postdot_items: AHashMap<Dotted<TI, TSP>, PostDotItems<TI, TD, TP, TSP, TS>>,
    added_postdot_items: AHashSet<Dotted<TI, TSP>>,
    // Maybe we could do a tree-like search to broaden the definition of leo items later.
    leo_items: AHashMap<ToBeCompletedItem<TI, TSP>, ToBeCompletedItem<TI, TSP>>,
    leo_items_buffer: Vec<ToBeCompletedItem<TI, TSP>>,
    already_predicted_nonterminals: FixedBitSet,
    finished: bool,
    config: EngineConfig,
    regex_start_config: regex_automata::util::start::Config,
    excepted_start_config: regex_automata::util::start::Config,
    temporary_change: bool,
}
#[allow(clippy::type_complexity)]
#[allow(clippy::too_many_arguments)]
impl<TI, TE, TD, TP, TSP, TS> EngineBase<TI, TE, TD, TP, TSP, TS>
where
    TI: Num
        + AsPrimitive<usize>
        + ConstOne
        + ConstZero
        + NumOps
        + NumAssign
        + std::cmp::PartialOrd
        + num::Bounded
        + std::convert::TryFrom<usize>,
    TI: Eq + std::hash::Hash + PartialEq,
    TE: AsPrimitive<usize>
        + crate::non_zero::ConstOne
        + Eq
        + std::hash::Hash
        + PartialEq
        + num::Bounded
        + std::convert::TryFrom<usize>
        + CheckedSub,
    TD: Num + AsPrimitive<usize> + ConstOne + ConstZero + Eq + std::hash::Hash + PartialEq,
    TP: Num + AsPrimitive<usize> + ConstOne + ConstZero + Eq + std::hash::Hash + PartialEq,
    TSP: Num + AsPrimitive<usize> + ConstOne + ConstZero + Eq + std::hash::Hash + PartialEq,
    TS: Num + AsPrimitive<usize> + ConstOne + ConstZero + Eq + std::hash::Hash + PartialEq,
    usize: num::traits::AsPrimitive<TI>
        + num::traits::AsPrimitive<TE>
        + num::traits::AsPrimitive<TD>
        + num::traits::AsPrimitive<TP>
        + num::traits::AsPrimitive<TSP>
        + num::traits::AsPrimitive<TS>,
{
    const STATE_ID_TYPE_SIZE: usize = std::mem::size_of::<TS>();
    const EXCEPTED_ID_TYPE_SIZE: usize = std::mem::size_of::<TE>();
    const STATE_ID_TYPE_BIT: usize = Self::STATE_ID_TYPE_SIZE * 8;
    const EXCEPTED_ID_TYPE_BIT: usize = Self::EXCEPTED_ID_TYPE_SIZE * 8;
    pub fn new(
        vocabulary: Arc<Vocabulary>,
        grammar: Arc<Grammar<TI, TE>>,
        config: EngineConfig,
    ) -> Result<Self, EngineError> {
        // Verify necessary conditions
        assert!(
            Self::STATE_ID_TYPE_SIZE <= USIZE_WIDTH,
            "state id type size {} is larger than usize width: {}",
            Self::STATE_ID_TYPE_SIZE,
            USIZE_WIDTH
        );
        Self::validate_ts_size_for_terminals(&grammar)?;
        Self::validate_ts_size_for_regexes(&grammar)?;
        Self::validate_ts_size_for_excepted(&grammar)?;
        // Init fields
        let allowed_first_bytes = ByteSet::with_capacity(u8::MAX as usize);
        let allowed_token_ids = FixedBitSet::with_capacity(vocabulary.get_vocab_size());
        let mut earley_sets = JaggedArray::new();
        earley_sets.new_row::<0>();
        let cache = AHashMap::default();
        let to_be_completed_items = AHashSet::default();
        let already_predicted_nonterminals =
            FixedBitSet::with_capacity(grammar.get_nonterminals_size());
        let regex_id_to_cache = AHashMap::default();
        let excepted_id_to_cache = AHashMap::default();
        let start = grammar.get_start_nonterminal_id();
        let postdot_items = AHashMap::default();
        let mut engine = Self {
            vocabulary,
            grammar,
            allowed_first_bytes,
            allowed_token_ids,
            earley_sets,
            cache,
            to_be_completed_items,
            already_predicted_nonterminals,
            config,
            regex_start_config: regex_automata::util::start::Config::new()
                .anchored(regex_automata::Anchored::Yes),
            excepted_start_config: regex_automata::util::start::Config::new()
                .anchored(regex_automata::Anchored::No),
            regex_id_to_cache,
            excepted_id_to_cache,
            postdot_items,
            leo_items: AHashMap::default(),
            finished: false,
            to_be_completed_items_buffer: AHashSet::default(),
            leo_items_buffer: Vec::new(),
            temporary_change: false,
            added_postdot_items: AHashSet::default(),
        };
        Self::predict_nonterminal(
            &engine.grammar,
            &mut engine.earley_sets,
            &mut engine.already_predicted_nonterminals,
            &engine.regex_start_config,
            &mut engine.regex_id_to_cache,
            &mut engine.excepted_id_to_cache,
            &engine.excepted_start_config,
            start,
            0,
        ); // init the first Earley set
        Self::predict(
            &engine.grammar,
            &mut engine.earley_sets,
            &engine.regex_start_config,
            &engine.excepted_start_config,
            &mut engine.regex_id_to_cache,
            &mut engine.excepted_id_to_cache,
            &mut engine.already_predicted_nonterminals,
        ); // run a full prediction for the first earley set
        engine.update_allowed_first_bytes();
        Self::update_postdot_items(
            &engine.grammar,
            &mut engine.earley_sets,
            &mut engine.postdot_items,
        );
        Ok(engine)
    }

    fn validate_ts_size_for_terminals(grammar: &Grammar<TI, TE>) -> Result<(), EngineError> {
        let terminals = grammar.get_id_to_terminals();
        let max: usize = (1 << Self::STATE_ID_TYPE_BIT) - 1;
        for i in 0..terminals.len() {
            let terminal = terminals.view::<1, 1>([i]);
            if terminal.len() > max {
                return Err(EngineError::TerminalTooLong(terminal.len(), max));
            }
        }
        Ok(())
    }

    fn validate_ts_size_for_regexes(grammar: &Grammar<TI, TE>) -> Result<(), EngineError> {
        let regexes = grammar.get_id_to_regexes();
        let max: usize = (1 << Self::STATE_ID_TYPE_BIT) - 1;
        for fsa in regexes {
            match fsa {
                FiniteStateAutomaton::Dfa(dfa) => {
                    if dfa.state_len() > max {
                        return Err(EngineError::RegexTooLarge(dfa.state_len(), max));
                    }
                }
                FiniteStateAutomaton::LazyDFA(_) => {
                    if LazyStateID::MAX > max {
                        return Err(EngineError::RegexTooLarge(LazyStateID::MAX, max));
                    }
                }
            }
        }
        Ok(())
    }

    fn validate_ts_size_for_excepted(grammar: &Grammar<TI, TE>) -> Result<(), EngineError> {
        let rules = grammar.get_rules();
        for i in 0..rules.len() {
            let productions = rules.view::<1, 2>([i]);
            for j in 0..productions.len() {
                let column = productions.view::<1, 1>([j]);
                for k in 0..column.len() {
                    let node = column[[k]];
                    if let LNFNode::EXCEPT(id, _) = node {
                        // repetition is verified in grammar
                        let fsa = grammar.get_excepted(id);
                        let max: usize =
                            (1 << (Self::STATE_ID_TYPE_BIT - Self::EXCEPTED_ID_TYPE_BIT)) - 1;
                        match fsa {
                            FiniteStateAutomaton::Dfa(dfa) => {
                                if dfa.state_len() > max {
                                    return Err(EngineError::ExceptedTooLarge(
                                        dfa.state_len(),
                                        max,
                                    ));
                                }
                            }
                            FiniteStateAutomaton::LazyDFA(_) => {
                                if LazyStateID::MAX > max {
                                    return Err(EngineError::ExceptedTooLarge(
                                        LazyStateID::MAX,
                                        max,
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Run prediction stage of Earley algorithm on last Earley set and current already_predicted_nonterminals content
    fn predict(
        grammar: &Grammar<TI, TE>,
        earley_sets: &mut EarleySets<TI, TD, TP, TSP, TS>,
        regex_start_config: &regex_automata::util::start::Config,
        excepted_start_config: &regex_automata::util::start::Config,
        regex_id_to_cache: &mut AHashMap<RegexID<TI>, Cache>,
        excepted_id_to_cache: &mut AHashMap<ExceptedID<TI>, Cache>,
        already_predicted_nonterminals: &mut FixedBitSet,
    ) {
        let earley_set_index = earley_sets.len() - 1;
        let mut earley_set_len = earley_sets.view::<1, 1>([earley_set_index]).len();
        let mut i = 0;
        while i < earley_set_len {
            let item = earley_sets[[earley_set_index, i]];
            let node = *grammar.get_node(
                item.nonterminal_id,
                item.dot_position,
                item.production_index,
            );
            if let LNFNode::Nonterminal(nonterminal_id) = node {
                earley_set_len += Self::predict_nonterminal(
                    grammar,
                    earley_sets,
                    already_predicted_nonterminals,
                    regex_start_config,
                    regex_id_to_cache,
                    excepted_id_to_cache,
                    excepted_start_config,
                    nonterminal_id,
                    earley_set_index,
                );
            }
            i += 1;
        }
        already_predicted_nonterminals.clear();
    }
    /// Predict one nonterminal according to Earley algorithm on the last Earley set.
    /// This function ensures no duplication happens.
    /// Returns earley set length increment due to prediction
    fn predict_nonterminal(
        grammar: &Grammar<TI, TE>,
        earley_sets: &mut EarleySets<TI, TD, TP, TSP, TS>,
        already_predicted_nonterminals: &mut FixedBitSet,
        regex_start_config: &regex_automata::util::start::Config,
        regex_id_to_cache: &mut AHashMap<RegexID<TI>, Cache>,
        excepted_id_to_cache: &mut AHashMap<ExceptedID<TI>, Cache>,
        excepted_start_config: &regex_automata::util::start::Config,
        nonterminal_id: NonterminalID<TI>,
        earley_set_index: usize,
    ) -> usize {
        let nid = nonterminal_id.0.as_();
        if !already_predicted_nonterminals.contains(nid) {
            already_predicted_nonterminals.insert(nid);
            let production_len = grammar.get_production_len(nonterminal_id);
            for j in 0..production_len {
                let production_index = j.as_();
                let new_item = EarleyItem {
                    nonterminal_id,
                    dot_position: TD::ZERO,
                    production_index,
                    start_position: earley_set_index.as_(),
                    state_id: match grammar.get_node(nonterminal_id, TD::ZERO, production_index) {
                        &LNFNode::RegexString(id) => {
                            let fsa = grammar.get_regex(id);
                            match fsa {
                                FiniteStateAutomaton::Dfa(dfa) => {
                                    // SAFETY: start_error will not happen since that will result in an error in Grammar::new() method
                                    let start = dfa.start_state(regex_start_config).unwrap();
                                    Self::from_dfa_state_id_to_state_id(start, dfa.stride2())
                                }
                                FiniteStateAutomaton::LazyDFA(dfa) => {
                                    // SAFETY: start_error will not happen since that will result in an error in Grammar::new() method
                                    let start = dfa
                                        .start_state(
                                            regex_id_to_cache.get_mut(&id).unwrap(),
                                            regex_start_config,
                                        )
                                        .unwrap();
                                    Self::from_ldfa_state_id_to_state_id(start)
                                }
                            }
                        }
                        LNFNode::EXCEPT(id, r) => {
                            let fsa = grammar.get_excepted(*id);
                            match fsa {
                                FiniteStateAutomaton::Dfa(dfa) => {
                                    // SAFETY: start_error will not happen since that will result in an error in Grammar::new() method
                                    let start = dfa.start_state(excepted_start_config).unwrap();
                                    match r {
                                        Some(r) => Self::from_dfa_state_id_to_state_id_with_r(
                                            start,
                                            dfa.stride2(),
                                            *r,
                                        ),
                                        None => Self::from_dfa_state_id_to_state_id(
                                            start,
                                            dfa.stride2(),
                                        ),
                                    }
                                }
                                FiniteStateAutomaton::LazyDFA(dfa) => {
                                    // SAFETY: start_error will not happen since that will result in an error in Grammar::new() method
                                    let start = dfa
                                        .start_state(
                                            excepted_id_to_cache.get_mut(id).unwrap(),
                                            excepted_start_config,
                                        )
                                        .unwrap();
                                    match r {
                                        Some(r) => {
                                            Self::from_ldfa_state_id_to_state_id_with_r(start, *r)
                                        }
                                        None => Self::from_ldfa_state_id_to_state_id(start),
                                    }
                                }
                            }
                        }
                        _ => TS::ZERO,
                    },
                };
                earley_sets.push_to_last_row(new_item);
            }
            production_len
        } else {
            0
        }
    }
    /// This function requires the last Earley set has been created and fully predicted.
    fn update_allowed_first_bytes(&mut self) {
        self.allowed_first_bytes.clear();
        let earley_set_index = self.earley_sets.len() - 1;
        let earley_set = self.earley_sets.view::<1, 1>([earley_set_index]).as_slice();
        for item in earley_set.iter() {
            let node = *self.grammar.get_node(
                item.nonterminal_id,
                item.dot_position,
                item.production_index,
            );
            match node {
                LNFNode::Terminal(terminal_id) => {
                    self.allowed_first_bytes
                        .insert(self.grammar.get_terminal(terminal_id)[0].as_());
                }
                LNFNode::RegexString(regex_id) => {
                    self.allowed_first_bytes
                        .union_with(self.grammar.get_first_bytes_from_regex(regex_id));
                }
                LNFNode::EXCEPT(excepted_id, _) => {
                    self.allowed_first_bytes
                        .union_with(self.grammar.get_first_bytes_from_excepted(excepted_id));
                }
                _ => {}
            }
        }
    }
    #[inline]
    fn item_should_be_completed(
        grammar: &Grammar<TI, TE>,
        nonterminal_id: NonterminalID<TI>,
        new_dot_position: TD,
        production_id: TP,
    ) -> bool
    where
        TP: Num + AsPrimitive<usize> + ConstOne + ConstZero,
        TD: Num + AsPrimitive<usize> + ConstOne + ConstZero,
    {
        let view = grammar.get_dotted_productions(nonterminal_id);
        if new_dot_position.as_() < view.len() {
            let view = view.view::<1, 1>([new_dot_position.as_()]);
            if production_id.as_() < view.len() {
                return false;
            }
        }
        true
    }

    /// This function requires the next Earley set has been created.
    fn advance_item(
        grammar: &Grammar<TI, TE>,
        to_be_completed_items: &mut AHashSet<ToBeCompletedItem<TI, TSP>>,
        earley_sets: &mut EarleySets<TI, TD, TP, TSP, TS>,
        item: EarleyItem<TI, TD, TP, TSP, TS>,
    ) {
        let new_dotted_position = item.dot_position + 1.as_();
        if Self::item_should_be_completed(
            grammar,
            item.nonterminal_id,
            new_dotted_position,
            item.production_index,
        ) {
            to_be_completed_items.insert(ToBeCompletedItem {
                nonterminal_id: item.nonterminal_id,
                start_position: item.start_position,
            });
        } else {
            let new_item = EarleyItem {
                nonterminal_id: item.nonterminal_id,
                dot_position: new_dotted_position,
                production_index: item.production_index,
                start_position: item.start_position,
                state_id: item.state_id,
            };
            earley_sets.push_to_last_row(new_item);
        }
    }
    #[inline]
    fn add_item_with_new_state(
        earley_sets: &mut EarleySets<TI, TD, TP, TSP, TS>,
        item: EarleyItem<TI, TD, TP, TSP, TS>,
        state_id: TS,
    ) {
        let new_item = EarleyItem {
            nonterminal_id: item.nonterminal_id,
            dot_position: item.dot_position,
            production_index: item.production_index,
            start_position: item.start_position,
            state_id,
        };
        earley_sets.push_to_last_row(new_item);
    }

    #[inline]
    fn from_state_id_to_index(state_id: TS) -> usize {
        state_id.as_()
    }
    #[inline]
    fn from_index_to_state_id(index: usize) -> TS {
        index.as_()
    }
    #[inline]
    fn from_dfa_state_id_to_state_id(state_id: StateID, stride2: usize) -> TS {
        // SAFETY: StateID is a u32 due to #[repr(transparent)] attribute
        let id: u32 = unsafe { std::mem::transmute(state_id) };
        // SAFETY: id is guaranteed to be representable as a state_id or an error will be returned in Self::new() method
        ((id >> stride2) as usize).as_()
    }
    #[inline]
    fn from_state_id_to_dfa_state_id(state_id: TS, stride2: usize) -> StateID {
        // SAFETY: StateID is a u32 due to #[repr(transparent)] attribute
        unsafe { std::mem::transmute((state_id.as_() << stride2) as u32) }
    }
    #[inline]
    fn from_dfa_state_id_to_state_id_with_r(state_id: StateID, stride2: usize, r: TE) -> TS {
        // SAFETY: state_id is a u32 due to #[repr(transparent)] attribute
        let id: u32 = unsafe { std::mem::transmute(state_id) };
        // SAFETY: id is guaranteed to be representable as a state_id or an error will be returned in Self::new() method
        let a = ((id >> stride2) as usize)
            + (r.as_() << (Self::STATE_ID_TYPE_BIT - Self::EXCEPTED_ID_TYPE_BIT));
        a.as_()
    }
    #[inline]
    fn from_state_id_to_dfa_state_id_with_r(state_id: TS, stride2: usize) -> (StateID, TE) {
        let id: u32 = state_id.as_() as u32;
        let r = (id >> (Self::STATE_ID_TYPE_BIT - Self::EXCEPTED_ID_TYPE_BIT)) as usize;
        // SAFETY: id is guaranteed to be representable as a state_id or an error will be returned in Self::new() method
        let state_id = ((id as usize
            - (r << (Self::STATE_ID_TYPE_BIT - Self::EXCEPTED_ID_TYPE_BIT)))
            << stride2) as u32;
        // SAFETY: StateID is a u32 due to #[repr(transparent)] attribute
        (unsafe { std::mem::transmute(state_id) }, r.as_())
    }
    #[inline]
    fn from_ldfa_state_id_to_state_id(state_id: LazyStateID) -> TS {
        // SAFETY: LazyStateID is a u32 due to #[repr(transparent)] attribute
        let id: u32 = unsafe { std::mem::transmute(state_id) };
        // SAFETY: id is guaranteed to be representable as a state_id or an error will be returned in Self::new() method
        (id as usize).as_()
    }
    #[inline]
    fn from_state_id_to_ldfa_state_id(state_id: TS) -> LazyStateID {
        // SAFETY: LazyStateID is a u32 due to #[repr(transparent)] attribute
        unsafe { std::mem::transmute((state_id.as_()) as u32) }
    }
    #[inline]
    fn from_ldfa_state_id_to_state_id_with_r(state_id: LazyStateID, r: TE) -> TS {
        // SAFETY: LazyStateID is a u32 due to #[repr(transparent)] attribute
        let id: u32 = unsafe { std::mem::transmute(state_id) };
        // SAFETY: id is guaranteed to be representable as a state_id or an error will be returned in Self::new() method
        let a = (id as usize)
            + (r.as_() << ((std::mem::size_of::<TS>() - std::mem::size_of::<TE>()) * 8));
        a.as_()
    }
    #[inline]
    fn from_state_id_to_ldfa_state_id_with_r(state_id: TS) -> (LazyStateID, TE) {
        let id: u32 = state_id.as_() as u32;
        let r = (id >> (Self::STATE_ID_TYPE_BIT - Self::EXCEPTED_ID_TYPE_BIT)) as usize;
        // SAFETY: id is guaranteed to be representable as a state_id or an error will be returned in Self::new() method
        let state_id =
            (id as usize - (r << (Self::STATE_ID_TYPE_BIT - Self::EXCEPTED_ID_TYPE_BIT))) as u32;
        // SAFETY: LazyStateID is a u32 due to #[repr(transparent)] attribute
        (unsafe { std::mem::transmute(state_id) }, r.as_())
    }
    // TODO: find some methods to reduce the repetitive code for regex and except!. Maybe we need a macro.
    fn scan(
        grammar: &Grammar<TI, TE>,
        earley_sets: &mut EarleySets<TI, TD, TP, TSP, TS>,
        to_be_completed_items: &mut AHashSet<ToBeCompletedItem<TI, TSP>>,
        regex_id_to_cache: &mut AHashMap<RegexID<TI>, Cache>,
        excepted_id_to_cache: &mut AHashMap<ExceptedID<TI>, Cache>,
        byte: u8,
    ) {
        let earley_set_index = earley_sets.len() - 1;
        let earley_set_len = earley_sets.view::<1, 1>([earley_set_index]).len();
        earley_sets.new_row::<0>();
        for i in 0..earley_set_len {
            let item = earley_sets[[earley_set_index, i]];
            let node = *grammar.get_node(
                item.nonterminal_id,
                item.dot_position,
                item.production_index,
            );
            match node {
                LNFNode::Terminal(terminal_id) => {
                    let terminal = grammar.get_terminal(terminal_id);
                    let index = Self::from_state_id_to_index(item.state_id);
                    if terminal[index] == byte {
                        let index = index + 1;
                        if index < terminal.len() {
                            let new_state_index = Self::from_index_to_state_id(index);
                            Self::add_item_with_new_state(earley_sets, item, new_state_index);
                        } else {
                            Self::advance_item(grammar, to_be_completed_items, earley_sets, item);
                        }
                    }
                }

                LNFNode::RegexString(regex_id) => {
                    let regex = grammar.get_regex(regex_id);
                    match regex {
                        FiniteStateAutomaton::Dfa(dfa) => {
                            let state_id =
                                Self::from_state_id_to_dfa_state_id(item.state_id, dfa.stride2());
                            let state_id = dfa.next_state(state_id, byte);
                            match utils::check_dfa_state_status(state_id, dfa) {
                                utils::FsaStateStatus::Accept => {
                                    Self::advance_item(
                                        grammar,
                                        to_be_completed_items,
                                        earley_sets,
                                        item,
                                    );
                                    let state_id = Self::from_dfa_state_id_to_state_id(
                                        state_id,
                                        dfa.stride2(),
                                    );
                                    Self::add_item_with_new_state(earley_sets, item, state_id);
                                }
                                utils::FsaStateStatus::Reject => {}
                                utils::FsaStateStatus::InProgress => {
                                    let state_id = Self::from_dfa_state_id_to_state_id(
                                        state_id,
                                        dfa.stride2(),
                                    );
                                    Self::add_item_with_new_state(earley_sets, item, state_id);
                                }
                            }
                        }
                        FiniteStateAutomaton::LazyDFA(ldfa) => {
                            let state_id = Self::from_state_id_to_ldfa_state_id(item.state_id);
                            let cache = regex_id_to_cache.get_mut(&regex_id).unwrap();
                            let state_id = ldfa.next_state(cache, state_id, byte).unwrap();
                            match utils::check_ldfa_state_status(state_id, cache, ldfa) {
                                utils::FsaStateStatus::Accept => {
                                    Self::advance_item(
                                        grammar,
                                        to_be_completed_items,
                                        earley_sets,
                                        item,
                                    );
                                    let state_id = Self::from_ldfa_state_id_to_state_id(state_id);
                                    Self::add_item_with_new_state(earley_sets, item, state_id);
                                }
                                utils::FsaStateStatus::Reject => {}
                                utils::FsaStateStatus::InProgress => {
                                    let state_id = Self::from_ldfa_state_id_to_state_id(state_id);
                                    Self::add_item_with_new_state(earley_sets, item, state_id);
                                }
                            }
                        }
                    }
                }
                LNFNode::Nonterminal(_) => {}
                LNFNode::EXCEPT(excepted_id, _) => {
                    let fsa = grammar.get_excepted(excepted_id);
                    match fsa {
                        FiniteStateAutomaton::Dfa(dfa) => {
                            let (state_id, r) = Self::from_state_id_to_dfa_state_id_with_r(
                                item.state_id,
                                dfa.stride2(),
                            );
                            let state_id = dfa.next_state(state_id, byte);
                            match utils::check_dfa_state_status(state_id, dfa) {
                                utils::FsaStateStatus::Accept => {}
                                utils::FsaStateStatus::Reject => {
                                    unreachable!("Except! should not reject")
                                }
                                utils::FsaStateStatus::InProgress => {
                                    if r.as_() == INVALID_REPETITION
                                    // repeat 1 or infinite times
                                    {
                                        Self::advance_item(
                                            grammar,
                                            to_be_completed_items,
                                            earley_sets,
                                            item,
                                        );
                                        let state_id = Self::from_dfa_state_id_to_state_id(
                                            state_id,
                                            dfa.stride2(),
                                        );
                                        Self::add_item_with_new_state(earley_sets, item, state_id);
                                        continue;
                                    }
                                    let r = r.checked_sub(&TE::ONE);
                                    match r {
                                        Some(r) => {
                                            // repetition is not exhausted
                                            Self::advance_item(
                                                grammar,
                                                to_be_completed_items,
                                                earley_sets,
                                                item,
                                            );
                                            let state_id =
                                                Self::from_dfa_state_id_to_state_id_with_r(
                                                    state_id,
                                                    dfa.stride2(),
                                                    r,
                                                );
                                            Self::add_item_with_new_state(
                                                earley_sets,
                                                item,
                                                state_id,
                                            );
                                        }
                                        None => {
                                            Self::advance_item(
                                                grammar,
                                                to_be_completed_items,
                                                earley_sets,
                                                item,
                                            );
                                        }
                                    }
                                }
                            }
                        }
                        FiniteStateAutomaton::LazyDFA(ldfa) => {
                            let (state_id, r) =
                                Self::from_state_id_to_ldfa_state_id_with_r(item.state_id);
                            let cache = excepted_id_to_cache.get_mut(&excepted_id).unwrap();
                            let state_id = ldfa.next_state(cache, state_id, byte).unwrap();
                            match utils::check_ldfa_state_status(state_id, cache, ldfa) {
                                utils::FsaStateStatus::Accept => {}
                                utils::FsaStateStatus::Reject => {
                                    unreachable!("Except! should not reject")
                                }
                                utils::FsaStateStatus::InProgress => {
                                    if r.as_() == INVALID_REPETITION
                                    // repeat 1 or infinite times
                                    {
                                        Self::advance_item(
                                            grammar,
                                            to_be_completed_items,
                                            earley_sets,
                                            item,
                                        );
                                        let state_id =
                                            Self::from_ldfa_state_id_to_state_id(state_id);
                                        Self::add_item_with_new_state(earley_sets, item, state_id);
                                        continue;
                                    }
                                    let r = r.checked_sub(&TE::ONE);
                                    match r {
                                        Some(r) => {
                                            // repetition is not exhausted
                                            Self::advance_item(
                                                grammar,
                                                to_be_completed_items,
                                                earley_sets,
                                                item,
                                            );
                                            let state_id =
                                                Self::from_ldfa_state_id_to_state_id_with_r(
                                                    state_id, r,
                                                );
                                            Self::add_item_with_new_state(
                                                earley_sets,
                                                item,
                                                state_id,
                                            );
                                        }
                                        None => {
                                            Self::advance_item(
                                                grammar,
                                                to_be_completed_items,
                                                earley_sets,
                                                item,
                                            );
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    fn update_postdot_items(
        grammar: &Grammar<TI, TE>,
        earley_sets: &mut EarleySets<TI, TD, TP, TSP, TS>,
        postdot_items: &mut AHashMap<Dotted<TI, TSP>, PostDotItems<TI, TD, TP, TSP, TS>>,
    ) {
        let earley_set_index = earley_sets.len() - 1;
        let earley_set = earley_sets.view::<1, 1>([earley_set_index]).as_slice();
        for item in earley_set.iter() {
            let node = *grammar.get_node(
                item.nonterminal_id,
                item.dot_position,
                item.production_index,
            );
            if let LNFNode::Nonterminal(nonterminal) = node {
                let postdot = Dotted {
                    postdot_nonterminal_id: nonterminal,
                    column: earley_set_index.as_(),
                };
                match postdot_items.entry(postdot) {
                    std::collections::hash_map::Entry::Occupied(mut entry) => {
                        let mut_ref = entry.get_mut();
                        match mut_ref {
                            &mut PostDotItems::LeoEligible(old_item) => {
                                *mut_ref = PostDotItems::NormalItems(vec![old_item, *item]);
                            }
                            PostDotItems::NormalItems(items) => {
                                items.push(*item);
                            }
                        }
                    }
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        entry.insert(PostDotItems::LeoEligible(*item));
                    }
                }
            }
        }
        for v in postdot_items.values_mut() {
            if let &mut PostDotItems::LeoEligible(item) = v {
                if !Self::item_should_be_completed(
                    grammar,
                    item.nonterminal_id,
                    item.dot_position + TD::ONE,
                    item.production_index,
                ) {
                    // not a leo item
                    *v = PostDotItems::NormalItems(vec![item]);
                }
            }
        }
    }
    #[allow(clippy::type_complexity)]
    fn try_leo_complete_item(
        leo_items_buffer: &mut Vec<ToBeCompletedItem<TI, TSP>>,
        leo_items: &mut AHashMap<ToBeCompletedItem<TI, TSP>, ToBeCompletedItem<TI, TSP>>,
        postdot_items: &AHashMap<Dotted<TI, TSP>, PostDotItems<TI, TD, TP, TSP, TS>>,
        mut topmost_item: ToBeCompletedItem<TI, TSP>,
    ) -> Option<ToBeCompletedItem<TI, TSP>> {
        if let Some(&leo_item) = leo_items.get(&topmost_item) {
            return Some(leo_item);
        }
        leo_items_buffer.clear();
        let mut is_leo = true;
        while is_leo {
            match postdot_items.get(&Dotted {
                postdot_nonterminal_id: topmost_item.nonterminal_id,
                column: topmost_item.start_position,
            }) {
                Some(v) => match v {
                    &PostDotItems::LeoEligible(leo_item) => {
                        leo_items_buffer.push(ToBeCompletedItem {
                            nonterminal_id: topmost_item.nonterminal_id,
                            start_position: topmost_item.start_position,
                        });
                        topmost_item = ToBeCompletedItem {
                            nonterminal_id: leo_item.nonterminal_id,
                            start_position: leo_item.start_position,
                        };
                    }
                    PostDotItems::NormalItems(_) => {
                        is_leo = false;
                    }
                },
                None => {
                    // We reach the beginning of the Earley sets
                    is_leo = false;
                }
            };
        }
        if leo_items_buffer.is_empty() {
            None
        } else {
            leo_items.reserve(leo_items_buffer.len());
            for &leo_item in leo_items_buffer.iter() {
                leo_items.insert(leo_item, topmost_item);
            }
            Some(topmost_item)
        }
    }
    #[allow(clippy::type_complexity)]
    fn earley_complete_one_item(
        grammar: &Grammar<TI, TE>,
        earley_sets: &mut EarleySets<TI, TD, TP, TSP, TS>,
        to_be_completed_item: ToBeCompletedItem<TI, TSP>,
        postdot_items: &AHashMap<Dotted<TI, TSP>, PostDotItems<TI, TD, TP, TSP, TS>>,
        to_be_completed_items_buffer: &mut AHashSet<ToBeCompletedItem<TI, TSP>>,
        is_finished: &mut bool,
    ) {
        match postdot_items.get(&Dotted {
            postdot_nonterminal_id: to_be_completed_item.nonterminal_id,
            column: to_be_completed_item.start_position,
        }) {
            Some(v) => match v {
                &PostDotItems::LeoEligible(_) => {
                    unreachable!("Leo item should already be handled")
                }
                PostDotItems::NormalItems(items) => {
                    for &item in items.iter() {
                        Self::advance_item(grammar, to_be_completed_items_buffer, earley_sets, item)
                    }
                }
            },
            None => {
                if grammar.get_start_nonterminal_id() == to_be_completed_item.nonterminal_id
                    && to_be_completed_item.start_position == TSP::ZERO
                {
                    *is_finished = true;
                }
            }
        }
    }

    fn complete(
        grammar: &Grammar<TI, TE>,
        earley_sets: &mut EarleySets<TI, TD, TP, TSP, TS>,
        to_be_completed_items: &mut AHashSet<ToBeCompletedItem<TI, TSP>>,
        to_be_completed_items_buffer: &mut AHashSet<ToBeCompletedItem<TI, TSP>>,
        leo_items: &mut AHashMap<ToBeCompletedItem<TI, TSP>, ToBeCompletedItem<TI, TSP>>,
        leo_items_buffer: &mut Vec<ToBeCompletedItem<TI, TSP>>,
        postdot_items: &AHashMap<Dotted<TI, TSP>, PostDotItems<TI, TD, TP, TSP, TS>>,
        finished: &mut bool,
    ) {
        to_be_completed_items_buffer.clear();
        while !to_be_completed_items.is_empty() {
            for item in to_be_completed_items.drain() {
                if let Some(topmost_item) =
                    Self::try_leo_complete_item(leo_items_buffer, leo_items, postdot_items, item)
                {
                    Self::earley_complete_one_item(
                        grammar,
                        earley_sets,
                        topmost_item,
                        postdot_items,
                        to_be_completed_items_buffer,
                        finished,
                    );
                } else {
                    Self::earley_complete_one_item(
                        grammar,
                        earley_sets,
                        item,
                        postdot_items,
                        to_be_completed_items_buffer,
                        finished,
                    );
                }
            }
            std::mem::swap(to_be_completed_items, to_be_completed_items_buffer);
        }
    }
}