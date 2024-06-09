//! This module contains the implementation of the [Engine](crate::engine::Engine) struct and is intended for advanced usages.
use ahash::{AHashMap, AHashSet};
use ebnf::regex::FiniteStateAutomaton;
use fixedbitset::FixedBitSet;
use jaggedarray::jagged_array::JaggedArray;
use jaggedarray::jagged_array::JaggedArrayViewTrait;
use nonmax::NonMaxU32;
use num::pow::Pow;
use num::{
    cast::AsPrimitive,
    traits::{ConstOne, ConstZero, NumAssign, NumOps},
    Num,
};
use regex_automata::dfa::Automaton;
use regex_automata::util::primitives::StateID;
use serde::Deserialize;
use serde::Serialize;
use std::fmt::Debug;
use std::sync::Arc;

use crate::engine_like::EngineLike;
use crate::grammar::INVALID_REPETITION;
use crate::utils;
use crate::utils::dispatch_by_dfa_state_status;
use crate::utils::ByteSet;
use crate::vocabulary::TokenIterItem;
use crate::{
    grammar::{Grammar, HIRNode, NonterminalID},
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

impl<TN, TD, TP, TSP, TS> EarleyItem<TN, TD, TP, TSP, TS>
where
    TN: Num
        + AsPrimitive<usize>
        + ConstOne
        + ConstZero
        + Eq
        + std::hash::Hash
        + PartialEq
        + std::fmt::Debug
        + PartialOrd
        + num::Bounded
        + num::traits::NumAssignOps
        + std::convert::TryFrom<usize>,
    TD: Num + AsPrimitive<usize> + ConstOne + ConstZero + Eq + std::hash::Hash + PartialEq,
    TP: Num + AsPrimitive<usize> + ConstOne + ConstZero + Eq + std::hash::Hash + PartialEq,
    TSP: Num + AsPrimitive<usize> + ConstOne + ConstZero + Eq + std::hash::Hash + PartialEq,
    TS: Num + AsPrimitive<usize> + ConstOne + ConstZero + Eq + std::hash::Hash + PartialEq,
    usize: num::traits::AsPrimitive<TN>
        + num::traits::AsPrimitive<TD>
        + num::traits::AsPrimitive<TP>
        + num::traits::AsPrimitive<TSP>
        + num::traits::AsPrimitive<TS>,
{
    fn to_debug_form<TE>(
        self,
        engine: &EngineBase<TN, TE, TD, TP, TSP, TS>,
    ) -> EarleyItemDebugStruct
    where
        TE: AsPrimitive<usize>
            + ConstOne
            + ConstZero
            + Num
            + std::convert::TryFrom<usize>
            + num::Bounded
            + NumAssign,
        usize: num::traits::AsPrimitive<TE>,
    {
        let dotted_productions =
            unsafe { engine.grammar.get_dotted_productions(self.nonterminal_id) };
        let mut dotted_rule = format!(
            "{} -> ",
            self.nonterminal_id.to_display_form(&engine.grammar)
        );
        for dot in 0..dotted_productions.len() {
            let production = dotted_productions.view::<1, 1>([dot]);
            if production.len() <= self.production_index.as_() {
                break;
            }
            if dot == self.dot_position.as_() {
                dotted_rule.push('.');
            }
            dotted_rule.push_str(
                &production[[self.production_index.as_()]].to_display_form(&engine.grammar),
            )
        }
        let state = if self.dot_position.as_() == dotted_productions.len() {
            dotted_rule.push('.');
            format!("[{}]", self.state_id.as_())
        } else {
            match engine.grammar.get_node(
                self.nonterminal_id,
                self.dot_position,
                self.production_index,
            ) {
                HIRNode::Terminal(_) => format!("[{}]", self.state_id.as_()),
                &HIRNode::RegexString(id) => {
                    match engine.grammar.get_regex(id) {
                        FiniteStateAutomaton::Dfa(dfa) => {
                            format!(
                            "[{}({})]",
                            self.state_id.as_(),
                            utils::check_dfa_state_status(
                                EngineBase::<TN,TE, TD, TP, TSP, TS>::from_state_id_to_dfa_state_id(
                                    self.state_id,
                                    dfa.stride2()
                                ),
                                dfa
                            )
                        )
                        }
                    }
                }
                &HIRNode::EXCEPT(id, r) => match engine.grammar.get_excepted(id) {
                    FiniteStateAutomaton::Dfa(dfa) => match r.as_() {
                        INVALID_REPETITION => format!(
                            "[{}({})]",
                            self.state_id.as_(),
                            utils::check_dfa_state_status(
                                EngineBase::<TN, TE, TD, TP, TSP, TS>::from_state_id_to_dfa_state_id(
                                    self.state_id,
                                    dfa.stride2()
                                ),
                                dfa
                            )
                        ),
                        _ => {
                            let (dfa_state_id, r) = EngineBase::<TN,TE, TD, TP, TSP, TS>::from_state_id_to_dfa_state_id_with_r(
                                self.state_id,
                                dfa.stride2(),
                            );
                            format!(
                                "[{}({}),R{}]",
                                self.state_id.as_(),
                                utils::check_dfa_state_status(dfa_state_id, dfa),
                                r.as_()
                            )
                        }
                    }
                },
                HIRNode::Nonterminal(_) => String::new(),
            }
        };
        EarleyItemDebugStruct {
            dotted_rule,
            start_position: self.start_position.as_(),
            state,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct EarleyItemDebugStruct {
    dotted_rule: String,
    start_position: usize,
    state: String,
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

impl<TN, TSP> ToBeCompletedItem<TN, TSP>
where
    TN: Num
        + AsPrimitive<usize>
        + ConstOne
        + ConstZero
        + Eq
        + std::hash::Hash
        + PartialEq
        + std::fmt::Debug
        + PartialOrd
        + num::Bounded
        + num::traits::NumAssignOps
        + std::convert::TryFrom<usize>,
    TSP: Num + AsPrimitive<usize> + ConstOne + ConstZero + Eq + std::hash::Hash + PartialEq,
    usize: num::traits::AsPrimitive<TN> + num::traits::AsPrimitive<TSP>,
{
    fn to_debug_form<TE>(self, grammar: &Grammar<TN, TE>) -> ToBeCompletedItemDebugStruct
    where
        TE: AsPrimitive<usize>
            + ConstOne
            + ConstZero
            + Num
            + std::convert::TryFrom<usize>
            + num::Bounded,
        usize: num::traits::AsPrimitive<TE>,
    {
        ToBeCompletedItemDebugStruct {
            nonterminal: self.nonterminal_id.to_display_form(grammar),
            start_position: self.start_position.as_(),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct ToBeCompletedItemDebugStruct {
    nonterminal: String,
    start_position: usize,
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

impl<TN, TSP> Dotted<TN, TSP>
where
    TN: Num
        + AsPrimitive<usize>
        + ConstOne
        + ConstZero
        + Eq
        + std::hash::Hash
        + PartialEq
        + std::fmt::Debug
        + PartialOrd
        + num::Bounded
        + num::traits::NumAssignOps
        + std::convert::TryFrom<usize>,
    TSP: Num + AsPrimitive<usize> + ConstOne + ConstZero + Eq + std::hash::Hash + PartialEq,
    usize: num::traits::AsPrimitive<TN> + num::traits::AsPrimitive<TSP>,
{
    fn to_debug_form<TE>(self, grammar: &Grammar<TN, TE>) -> DottedDebugStruct
    where
        TE: AsPrimitive<usize>
            + ConstOne
            + ConstZero
            + Num
            + std::convert::TryFrom<usize>
            + num::Bounded,
        usize: num::traits::AsPrimitive<TE>,
    {
        DottedDebugStruct {
            postdot_nonterminal: self.postdot_nonterminal_id.to_display_form(grammar),
            column: self.column.as_(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct DottedDebugStruct {
    postdot_nonterminal: String,
    column: usize,
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

impl<TN, TD, TP, TSP, TS> PostDotItems<TN, TD, TP, TSP, TS>
where
    TN: Num
        + AsPrimitive<usize>
        + ConstOne
        + ConstZero
        + Eq
        + std::hash::Hash
        + PartialEq
        + std::fmt::Debug
        + PartialOrd
        + num::Bounded
        + num::traits::NumAssignOps
        + std::convert::TryFrom<usize>,
    TD: Num + AsPrimitive<usize> + ConstOne + ConstZero + Eq + std::hash::Hash + PartialEq,
    TP: Num + AsPrimitive<usize> + ConstOne + ConstZero + Eq + std::hash::Hash + PartialEq,
    TSP: Num + AsPrimitive<usize> + ConstOne + ConstZero + Eq + std::hash::Hash + PartialEq,
    TS: Num + AsPrimitive<usize> + ConstOne + ConstZero + Eq + std::hash::Hash + PartialEq,
    usize: num::traits::AsPrimitive<TN>
        + num::traits::AsPrimitive<TD>
        + num::traits::AsPrimitive<TP>
        + num::traits::AsPrimitive<TSP>
        + num::traits::AsPrimitive<TS>,
{
    fn to_debug_form<TE>(
        &self,
        engine: &EngineBase<TN, TE, TD, TP, TSP, TS>,
    ) -> PostDotItemsDebugStruct
    where
        TE: AsPrimitive<usize>
            + ConstOne
            + ConstZero
            + Num
            + std::convert::TryFrom<usize>
            + num::Bounded
            + NumAssign,
        usize: num::traits::AsPrimitive<TE>,
    {
        match self {
            PostDotItems::LeoEligible(item) => {
                PostDotItemsDebugStruct::LeoEligible(item.to_debug_form(engine))
            }
            PostDotItems::NormalItems(items) => PostDotItemsDebugStruct::NormalItems(
                items.iter().map(|x| x.to_debug_form(engine)).collect(),
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum PostDotItemsDebugStruct {
    LeoEligible(EarleyItemDebugStruct),
    NormalItems(Vec<EarleyItemDebugStruct>),
}
/// The specific config of the engine
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct EngineConfig {
    /// Whether the cache is enabled. Caching speeds up the engine if any of the following conditions are met:
    /// 1. The grammar is 'simple'. What exactly constitutes a simple grammar is not well defined at the moment but
    /// grammars purely made of left recursive rules, right recursive rules and/or regular rules should be simple.
    /// 2. The grammar is reused multiple times for inputs of similar lengths.
    /// It is enabled by default.
    pub cache_enabled: bool,
    /// Whether the compaction is enabled. Compaction reduces the memory usage of the engine and
    /// should not affect the performance significantly. In particular, usually caching requires compaction to be effective.
    /// It is enabled by default.
    pub compaction_enabled: bool,
}
/// The error type for errors in engine creation.
#[derive(Debug, thiserror::Error)]
pub enum EngineBaseError {
    #[error(
        "Terminal length {0} exceeds {1}, the maximum terminal length allowed by current size of StateID(TS).
     Consider reducing terminal length or use larger StateID(TS)."
    )]
    /// The terminal length exceeds the maximum terminal length allowed by the current size of StateID(TS).
    TerminalTooLong(usize, usize),
    #[error(
        "Regex length {0} exceeds {1}, the maximum regex length allowed by current size of StateID(TS).
     Consider reducing regex states or use larger StateID(TS)."
    )]
    /// The regex length exceeds the maximum regex length allowed by the current size of StateID(TS).s
    RegexTooLarge(usize, usize),
    #[error(
        "Except! length {0} exceeds {1}, the maximum excepted length allowed by current size of StateID(TS).
     Consider reducing excepted terminals, use larger StateID(TS) or less repetition."
    )]
    /// The excepted length exceeds the maximum excepted length allowed by the current size of StateID(TS).
    ExceptedTooLarge(usize, usize),
    #[error(
        "Repetition in regex {0} exceeds {1}, the maximum repetition allowed by current size of StateID(TS).
     Consider reducing repetition or use larger StateID(TS)."
    )]
    /// The repetition in regex exceeds the maximum repetition allowed by the current size of StateID(TS).
    RepetitionInExceptedTooLarge(usize, usize),
}
#[allow(clippy::type_complexity)]
#[derive(Clone)]
/// The low-level engine struct that implement a variant of the Earley recognizer.
pub struct EngineBase<TI, TE, TD, TP, TSP, TS>
where
    TI: Num
        + AsPrimitive<usize>
        + ConstOne
        + ConstZero
        + Eq
        + std::hash::Hash
        + PartialEq
        + std::fmt::Debug
        + PartialOrd
        + num::Bounded
        + std::convert::TryFrom<usize>
        + NumAssign,
    TE: AsPrimitive<usize>
        + ConstOne
        + ConstZero
        + Num
        + std::convert::TryFrom<usize>
        + num::Bounded,
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
    to_be_completed_items: AHashSet<ToBeCompletedItem<TI, TSP>>,
    to_be_completed_items_buffer: AHashSet<ToBeCompletedItem<TI, TSP>>,
    deduplication_buffer: AHashSet<EarleyItem<TI, TD, TP, TSP, TS>>,
    // Maybe a smallvec will be better. Profiling is needed to make a decision.
    // I feel like copying the item is better than add a reference to the item since the item is relatively small(<=16 bytes)
    postdot_items: AHashMap<Dotted<TI, TSP>, PostDotItems<TI, TD, TP, TSP, TS>>,
    postdot_items_since_last_commit: AHashSet<Dotted<TI, TSP>>,
    // Maybe we could do a tree-like search to broaden the definition of leo items later.
    leo_items: AHashMap<ToBeCompletedItem<TI, TSP>, ToBeCompletedItem<TI, TSP>>,
    leo_items_buffer: Vec<ToBeCompletedItem<TI, TSP>>,
    already_predicted_nonterminals: FixedBitSet,
    finished: bool,
    config: EngineConfig,
    regex_start_config: regex_automata::util::start::Config,
    excepted_start_config: regex_automata::util::start::Config,
}

impl<TI, TE, TD, TP, TSP, TS> Debug for EngineBase<TI, TE, TD, TP, TSP, TS>
where
    TI: Num
        + AsPrimitive<usize>
        + ConstOne
        + ConstZero
        + Eq
        + std::hash::Hash
        + PartialEq
        + std::fmt::Debug
        + PartialOrd
        + num::Bounded
        + std::convert::TryFrom<usize>
        + NumAssign,
    TE: AsPrimitive<usize>
        + ConstOne
        + ConstZero
        + Num
        + std::convert::TryFrom<usize>
        + num::Bounded
        + NumAssign
        + Debug,
    TD: Num + AsPrimitive<usize> + ConstOne + ConstZero + Eq + std::hash::Hash + PartialEq,
    TP: Num + AsPrimitive<usize> + ConstOne + ConstZero + Eq + std::hash::Hash + PartialEq,
    TSP: Num + AsPrimitive<usize> + ConstOne + ConstZero + Eq + std::hash::Hash + PartialEq,
    TS: Num + AsPrimitive<usize> + ConstOne + ConstZero + Eq + std::hash::Hash + PartialEq,
    usize: num::traits::AsPrimitive<TI>
        + num::traits::AsPrimitive<TD>
        + num::traits::AsPrimitive<TP>
        + num::traits::AsPrimitive<TSP>
        + num::traits::AsPrimitive<TE>
        + num::traits::AsPrimitive<TS>,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineBase")
            .field("grammar", &self.grammar)
            .field(
                "allowed_first_bytes",
                &utils::get_display_form_from_bitset_on_stack(&self.allowed_first_bytes),
            )
            .field("allowed_token_ids", {
                &self
                    .allowed_token_ids
                    .ones()
                    .map(|x| {
                        format!(
                            "{}[{}]",
                            self.vocabulary
                                .get_token_string_from_token_id(x as u32)
                                .unwrap(),
                            x
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .field(
                "earley_sets",
                &self.get_display_form_from_earley_sets(&self.earley_sets),
            )
            .field(
                "cache",
                &self
                    .cache
                    .iter()
                    .map(|(k, v)| (self.get_display_form_from_earley_sets(k), v))
                    .collect::<Vec<_>>(),
            )
            .field("to_be_completed_items", {
                let mut a = self
                    .to_be_completed_items
                    .iter()
                    .map(|x| x.to_debug_form(&self.grammar))
                    .collect::<Vec<_>>();
                a.sort();
                &Box::new(a)
            })
            .field("to_be_completed_items_buffer", {
                let mut a = self
                    .to_be_completed_items_buffer
                    .iter()
                    .map(|x| x.to_debug_form(&self.grammar))
                    .collect::<Vec<_>>();
                a.sort_by_key(|x| x.start_position);
                &Box::new(a)
            })
            .field("deduplication_buffer", {
                let mut a = self
                    .deduplication_buffer
                    .iter()
                    .map(|x| x.to_debug_form(self))
                    .collect::<Vec<_>>();
                a.sort_by_key(|x| x.start_position);
                &Box::new(a)
            })
            .field("postdot_items", {
                let mut a = self
                    .postdot_items
                    .iter()
                    .map(|(k, v)| (k.to_debug_form(&self.grammar), v.to_debug_form(self)))
                    .collect::<Vec<_>>();
                a.sort_by_cached_key(|(k, _)| k.clone());
                &Box::new(a)
            })
            .field("postdot_items_since_last_commit", {
                let mut a = self
                    .postdot_items_since_last_commit
                    .iter()
                    .map(|x| x.to_debug_form(&self.grammar))
                    .collect::<Vec<_>>();
                a.sort();
                &Box::new(a)
            })
            .field("leo_items", {
                let mut a = self
                    .leo_items
                    .iter()
                    .map(|(k, v)| {
                        (
                            k.to_debug_form(&self.grammar),
                            v.to_debug_form(&self.grammar),
                        )
                    })
                    .collect::<Vec<_>>();
                a.sort_by_cached_key(|(k, _)| k.clone());
                &Box::new(a)
            })
            .field(
                "leo_items_buffer",
                &self
                    .leo_items_buffer
                    .iter()
                    .map(|x| x.to_debug_form(&self.grammar))
                    .collect::<Vec<_>>(),
            )
            .field(
                "already_predicted_nonterminals",
                &utils::get_display_form_from_bitset(&self.already_predicted_nonterminals),
            )
            .field("finished", &self.finished)
            .field("config", &self.config)
            .field("regex_start_config", &self.regex_start_config)
            .field("excepted_start_config", &self.excepted_start_config)
            .finish()
    }
}

#[allow(clippy::type_complexity)]
#[allow(clippy::too_many_arguments)]
impl<TI, TE, TD, TP, TSP, TS> EngineBase<TI, TE, TD, TP, TSP, TS>
where
    TI: Num
        + AsPrimitive<usize>
        + ConstOne
        + ConstZero
        + Eq
        + std::hash::Hash
        + PartialEq
        + std::fmt::Debug
        + PartialOrd
        + num::Bounded
        + num::traits::NumAssignOps
        + std::convert::TryFrom<usize>,
    TE: AsPrimitive<usize>
        + ConstOne
        + ConstZero
        + Num
        + std::convert::TryFrom<usize>
        + num::Bounded
        + NumAssign,
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
    /// Create a new [EngineBase](crate::engine_base::EngineBase).
    ///
    /// # Arguments
    ///
    /// * `vocabulary` - The vocabulary of the language model.
    /// * `grammar` - The grammar of the language model.
    /// * `config` - The specific config of the engine.
    ///
    /// # Returns
    ///
    /// A new [EngineBase](crate::engine_base::EngineBase) instance.
    ///
    /// # Errors
    ///
    /// Returns an error if the terminal length, regex length, excepted length
    /// or repetition in regex exceeds the maximum allowed by the current size of StateID(TS).
    pub fn new(
        vocabulary: Arc<Vocabulary>,
        grammar: Arc<Grammar<TI, TE>>,
        config: EngineConfig,
    ) -> Result<Self, EngineBaseError> {
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
        let allowed_token_ids = FixedBitSet::with_capacity(vocabulary.get_vocab_size() + 1);
        let earley_sets = JaggedArray::new();
        let cache = AHashMap::default();
        let to_be_completed_items = AHashSet::default();
        let already_predicted_nonterminals =
            FixedBitSet::with_capacity(grammar.get_nonterminals_size());
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
            postdot_items,
            leo_items: AHashMap::default(),
            finished: false,
            to_be_completed_items_buffer: AHashSet::default(),
            leo_items_buffer: Vec::new(),
            postdot_items_since_last_commit: AHashSet::default(),
            deduplication_buffer: AHashSet::default(),
        };
        engine.reset();
        Ok(engine)
    }

    fn get_display_form_from_earley_sets(
        &self,
        sets: &EarleySets<TI, TD, TP, TSP, TS>,
    ) -> Vec<Vec<EarleyItemDebugStruct>> {
        let mut res = Vec::with_capacity(sets.len());
        for i in 0..sets.len() {
            let set = sets.view::<1, 1>([i]);
            let mut set_res = Vec::with_capacity(set.len());
            for j in 0..set.len() {
                set_res.push(set[[j]].to_debug_form(self));
            }
            res.push(set_res);
        }
        res
    }

    fn validate_ts_size_for_terminals(grammar: &Grammar<TI, TE>) -> Result<(), EngineBaseError> {
        let terminals = grammar.get_id_to_terminals();
        let max: usize = (1 << Self::STATE_ID_TYPE_BIT) - 1;
        for i in 0..terminals.len() {
            let terminal = terminals.view::<1, 1>([i]);
            if terminal.len() > max {
                return Err(EngineBaseError::TerminalTooLong(terminal.len(), max));
            }
        }
        Ok(())
    }

    fn validate_ts_size_for_regexes(grammar: &Grammar<TI, TE>) -> Result<(), EngineBaseError> {
        let regexes = grammar.get_id_to_regexes();
        let max: usize = (1 << Self::STATE_ID_TYPE_BIT) - 1;
        for fsa in regexes {
            match fsa {
                FiniteStateAutomaton::Dfa(dfa) => {
                    if dfa.state_len() > max {
                        return Err(EngineBaseError::RegexTooLarge(dfa.state_len(), max));
                    }
                }
            }
        }
        Ok(())
    }

    fn validate_ts_size_for_excepted(grammar: &Grammar<TI, TE>) -> Result<(), EngineBaseError> {
        let rules = grammar.get_rules();
        for i in 0..rules.len() {
            let productions = rules.view::<1, 2>([i]);
            for j in 0..productions.len() {
                let column = productions.view::<1, 1>([j]);
                for k in 0..column.len() {
                    let node = column[[k]];
                    if let HIRNode::EXCEPT(id, _) = node {
                        // repetition is verified in grammar
                        let fsa = grammar.get_excepted(id);
                        let max: usize =
                            (1 << (Self::STATE_ID_TYPE_BIT - Self::EXCEPTED_ID_TYPE_BIT)) - 1;
                        match fsa {
                            FiniteStateAutomaton::Dfa(dfa) => {
                                if dfa.state_len() > max {
                                    return Err(EngineBaseError::ExceptedTooLarge(
                                        dfa.state_len(),
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
        already_predicted_nonterminals: &mut FixedBitSet,
    ) {
        let earley_set_index = earley_sets.len() - 1;
        let mut earley_set_len =
            unsafe { earley_sets.view_unchecked::<1, 1>([earley_set_index]).len() };
        let mut i = 0;
        while i < earley_set_len {
            let item = unsafe { *earley_sets.get_unchecked([earley_set_index, i]) };
            // SAFETY: Earley algorithm guarantees item is a valid index
            let node = unsafe {
                *grammar.get_node_unchecked(
                    item.nonterminal_id,
                    item.dot_position,
                    item.production_index,
                )
            };
            if let HIRNode::Nonterminal(nonterminal_id) = node {
                earley_set_len += Self::predict_nonterminal(
                    grammar,
                    earley_sets,
                    already_predicted_nonterminals,
                    regex_start_config,
                    excepted_start_config,
                    nonterminal_id,
                    earley_set_index,
                );
            }
            i += 1;
        }
        already_predicted_nonterminals.clear();
    }

    fn initialize_state_id_based_on_node(
        grammar: &Grammar<TI, TE>,
        regex_start_config: &regex_automata::util::start::Config,
        excepted_start_config: &regex_automata::util::start::Config,
        node: HIRNode<TI, TE>,
    ) -> TS {
        match node {
            HIRNode::RegexString(id) => {
                let fsa = grammar.get_regex(id);
                match fsa {
                    FiniteStateAutomaton::Dfa(dfa) => {
                        // SAFETY: start_error will not happen since that will result in an error in Grammar::new() method
                        let start = dfa.start_state(regex_start_config).unwrap();
                        Self::from_dfa_state_id_to_state_id(start, dfa.stride2())
                    }
                }
            }
            HIRNode::EXCEPT(id, r) => {
                let fsa = grammar.get_excepted(id);
                match fsa {
                    FiniteStateAutomaton::Dfa(dfa) => {
                        // SAFETY: start_error will not happen since that will result in an error in Grammar::new() method
                        let start = dfa.start_state(excepted_start_config).unwrap();
                        match r.as_() {
                            INVALID_REPETITION => {
                                Self::from_dfa_state_id_to_state_id(start, dfa.stride2())
                            }
                            _ => {
                                Self::from_dfa_state_id_to_state_id_with_r(start, dfa.stride2(), r)
                            }
                        }
                    }
                }
            }
            _ => TS::ZERO,
        }
    }

    /// Predict one nonterminal according to Earley algorithm on the last Earley set.
    /// This function ensures no duplication happens.
    /// Returns earley set length increment due to prediction
    fn predict_nonterminal(
        grammar: &Grammar<TI, TE>,
        earley_sets: &mut EarleySets<TI, TD, TP, TSP, TS>,
        already_predicted_nonterminals: &mut FixedBitSet,
        regex_start_config: &regex_automata::util::start::Config,
        excepted_start_config: &regex_automata::util::start::Config,
        nonterminal_id: NonterminalID<TI>,
        earley_set_index: usize,
    ) -> usize {
        let nid = nonterminal_id.0.as_();
        if !already_predicted_nonterminals.contains(nid) {
            already_predicted_nonterminals.insert(nid);
            let production_len = grammar.get_production_len(nonterminal_id);
            earley_sets.buffer_reserve(earley_sets.buffer_len() + production_len);
            for j in 0..production_len {
                let production_index = j.as_();
                let new_item = EarleyItem {
                    nonterminal_id,
                    dot_position: TD::ZERO,
                    production_index,
                    start_position: earley_set_index.as_(),
                    state_id: Self::initialize_state_id_based_on_node(
                        grammar,
                        regex_start_config,
                        excepted_start_config,
                        unsafe {
                            *grammar.get_node_unchecked(nonterminal_id, TD::ZERO, production_index)
                        },
                    ),
                };
                unsafe { earley_sets.push_to_last_row_unchecked(new_item) };
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
                HIRNode::Terminal(terminal_id) => {
                    self.allowed_first_bytes
                        .insert(self.grammar.get_terminal(terminal_id)[0].as_());
                }
                HIRNode::RegexString(regex_id) => {
                    self.allowed_first_bytes
                        .union_with(self.grammar.get_first_bytes_from_regex(regex_id));
                }
                HIRNode::EXCEPT(excepted_id, _) => {
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
        // SAFETY: nonterminal_id is guaranteed to be valid
        let view = unsafe { grammar.get_dotted_productions(nonterminal_id) };
        if new_dot_position.as_() < view.len() {
            let view = unsafe { view.view_unchecked::<1, 1>([new_dot_position.as_()]) };
            if production_id.as_() < view.len() {
                return false;
            }
        }
        true
    }

    fn advance_item<T>(
        grammar: &Grammar<TI, TE>,
        to_be_completed_items: &mut AHashSet<ToBeCompletedItem<TI, TSP>>,
        regex_start_config: &regex_automata::util::start::Config,
        excepted_start_config: &regex_automata::util::start::Config,
        add_to_earley_set: T,
        mut item: EarleyItem<TI, TD, TP, TSP, TS>,
    ) where
        T: FnOnce(EarleyItem<TI, TD, TP, TSP, TS>),
    {
        let new_dotted_position = item.dot_position + TD::ONE;
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
            item.dot_position = new_dotted_position;
            item.state_id = Self::initialize_state_id_based_on_node(
                grammar,
                regex_start_config,
                excepted_start_config,
                unsafe {
                    *grammar.get_node_unchecked(
                        item.nonterminal_id,
                        new_dotted_position,
                        item.production_index,
                    )
                },
            );
            add_to_earley_set(item);
        }
    }
    #[inline]
    fn advance_item_normal(
        grammar: &Grammar<TI, TE>,
        earley_sets: &mut EarleySets<TI, TD, TP, TSP, TS>,
        to_be_completed_items: &mut AHashSet<ToBeCompletedItem<TI, TSP>>,
        regex_start_config: &regex_automata::util::start::Config,
        excepted_start_config: &regex_automata::util::start::Config,
        item: EarleyItem<TI, TD, TP, TSP, TS>,
    ) {
        Self::advance_item(
            grammar,
            to_be_completed_items,
            regex_start_config,
            excepted_start_config,
            |new_item| {
                earley_sets.push_to_last_row(new_item);
            },
            item,
        );
    }

    #[inline]
    unsafe fn advance_item_normal_unchecked(
        grammar: &Grammar<TI, TE>,
        earley_sets: &mut EarleySets<TI, TD, TP, TSP, TS>,
        to_be_completed_items: &mut AHashSet<ToBeCompletedItem<TI, TSP>>,
        regex_start_config: &regex_automata::util::start::Config,
        excepted_start_config: &regex_automata::util::start::Config,
        item: EarleyItem<TI, TD, TP, TSP, TS>,
    ) {
        Self::advance_item(
            grammar,
            to_be_completed_items,
            regex_start_config,
            excepted_start_config,
            |new_item| {
                earley_sets.push_to_last_row_unchecked(new_item);
            },
            item,
        );
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
        let id: usize = state_id.as_();
        if Self::EXCEPTED_ID_TYPE_BIT == 0 { // avoid overflow
            return (Self::from_state_id_to_dfa_state_id(state_id, stride2), TE::ZERO);
        }
        let r = id >> (Self::STATE_ID_TYPE_BIT - Self::EXCEPTED_ID_TYPE_BIT);
        // SAFETY: id is guaranteed to be representable as a state_id or an error will be returned in Self::new() method
        let state_id = ((id
            - (r << (Self::STATE_ID_TYPE_BIT - Self::EXCEPTED_ID_TYPE_BIT)))
            << stride2) as u32;
        // SAFETY: StateID is a u32 due to #[repr(transparent)] attribute
        (unsafe { std::mem::transmute(state_id) }, r.as_())
    }

    fn scan(
        grammar: &Grammar<TI, TE>,
        earley_sets: &mut EarleySets<TI, TD, TP, TSP, TS>,
        to_be_completed_items: &mut AHashSet<ToBeCompletedItem<TI, TSP>>,
        regex_start_config: &regex_automata::util::start::Config,
        excepted_start_config: &regex_automata::util::start::Config,
        byte: u8,
    ) {
        let earley_set_index: usize = earley_sets.len() - 1; // Interestingly usize seems to be faster than i32
        let earley_set_len =
            unsafe { earley_sets.view_unchecked::<1, 1>([earley_set_index]).len() };
        earley_sets.new_row::<0>();
        earley_sets.buffer_reserve(earley_sets.buffer_len() + earley_set_len * 2);
        for i in 0..earley_set_len {
            // SAFETY: 0<i<earley_set_len and earley sets is never empty ensures the index is valid
            let mut item = unsafe { *earley_sets.get_unchecked([earley_set_index, i]) };
            let node = unsafe {
                *grammar.get_node_unchecked(
                    item.nonterminal_id,
                    item.dot_position,
                    item.production_index,
                )
            };
            match node {
                HIRNode::Terminal(terminal_id) => {
                    let terminal = unsafe { grammar.get_terminal_unchecked(terminal_id) };
                    let mut index = Self::from_state_id_to_index(item.state_id);
                    if unsafe { *terminal.get_unchecked(index) } == byte {
                        index += 1;
                        if index != terminal.len() {
                            // interestingly faster than <
                            let new_state_index = Self::from_index_to_state_id(index);
                            item.state_id = new_state_index;
                            earley_sets.push_to_last_row(item);
                        } else {
                            unsafe {
                                Self::advance_item_normal_unchecked(
                                    grammar,
                                    earley_sets,
                                    to_be_completed_items,
                                    regex_start_config,
                                    excepted_start_config,
                                    item,
                                )
                            };
                        }
                    }
                }
                HIRNode::RegexString(regex_id) => {
                    let regex = unsafe { grammar.get_regex_unchecked(regex_id) };
                    match regex {
                        FiniteStateAutomaton::Dfa(dfa) => {
                            let mut state_id =
                                Self::from_state_id_to_dfa_state_id(item.state_id, dfa.stride2());
                            state_id = dfa.next_state(state_id, byte);
                            dispatch_by_dfa_state_status!(
                                state_id,
                                dfa,
                                accept=>{
                                    unsafe{Self::advance_item_normal_unchecked(
                                        grammar,
                                        earley_sets,
                                        to_be_completed_items,
                                        regex_start_config,
                                        excepted_start_config,
                                        item,
                                    )};
                                    let state_id = Self::from_dfa_state_id_to_state_id(
                                        state_id,
                                        dfa.stride2(),
                                    );
                                    item.state_id = state_id;
                                    unsafe{earley_sets.push_to_last_row_unchecked(item)};
                                },
                                reject=>{},
                                in_progress=>
                                {
                                    let state_id = Self::from_dfa_state_id_to_state_id(
                                        state_id,
                                        dfa.stride2(),
                                    );
                                    item.state_id = state_id;
                                    unsafe{earley_sets.push_to_last_row_unchecked(item)};
                                }
                            );
                        }
                    }
                }
                HIRNode::EXCEPT(excepted_id, _) => {
                    let fsa = grammar.get_excepted(excepted_id);
                    match fsa {
                        FiniteStateAutomaton::Dfa(dfa) => {
                            let (state_id, mut r) = Self::from_state_id_to_dfa_state_id_with_r(
                                item.state_id,
                                dfa.stride2(),
                            );
                            let state_id = dfa.next_state(state_id, byte);
                            dispatch_by_dfa_state_status!(
                                state_id,
                                dfa,
                                accept=>{},
                                reject=>{ unreachable!("Except! should not reject") },
                                in_progress=>{
                                    if r == INVALID_REPETITION.as_()
                                    // repeat 1 or infinite times
                                    {
                                        unsafe{Self::advance_item_normal_unchecked(
                                            grammar,
                                            earley_sets,
                                            to_be_completed_items,
                                            regex_start_config,
                                            excepted_start_config,
                                            item,
                                        )};
                                        let state_id = Self::from_dfa_state_id_to_state_id(
                                            state_id,
                                            dfa.stride2(),
                                        );
                                        item.state_id = state_id;
                                        unsafe{earley_sets.push_to_last_row_unchecked(item)};
                                    }
                                    else{
                                        r -= TE::ONE;
                                        match r.as_() {
                                            INVALID_REPETITION => {
                                                unsafe{Self::advance_item_normal_unchecked(
                                                    grammar,
                                                    earley_sets,
                                                    to_be_completed_items,
                                                    regex_start_config,
                                                    excepted_start_config,
                                                    item,
                                                )};
                                            }
                                            _ => {
                                                // repetition is not exhausted
                                                unsafe{Self::advance_item_normal_unchecked(
                                                    grammar,
                                                    earley_sets,
                                                    to_be_completed_items,
                                                    regex_start_config,
                                                    excepted_start_config,
                                                    item,
                                                )};
                                                let state_id =
                                                    Self::from_dfa_state_id_to_state_id_with_r(
                                                        state_id,
                                                        dfa.stride2(),
                                                        r,
                                                    );
                                                item.state_id = state_id;
                                                unsafe{earley_sets.push_to_last_row_unchecked(item)};
                                            }
                                        }
                                    }
                                }
                            );
                        }
                    }
                }
                HIRNode::Nonterminal(_) => {}
            }
        }
    }
    fn update_postdot_items(
        grammar: &Grammar<TI, TE>,
        earley_sets: &mut EarleySets<TI, TD, TP, TSP, TS>,
        postdot_items: &mut AHashMap<Dotted<TI, TSP>, PostDotItems<TI, TD, TP, TSP, TS>>,
        added_postdot_items: &mut AHashSet<Dotted<TI, TSP>>,
    ) {
        let earley_set_index = earley_sets.len() - 1;
        let earley_set = unsafe {
            earley_sets
                .view_unchecked::<1, 1>([earley_set_index])
                .as_slice()
        };
        for item in earley_set.iter() {
            let node = *unsafe {
                grammar.get_node_unchecked(
                    item.nonterminal_id,
                    item.dot_position,
                    item.production_index,
                )
            };
            if let HIRNode::Nonterminal(nonterminal) = node {
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
                        added_postdot_items.insert(postdot);
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
        to_be_completed_item: ToBeCompletedItem<TI, TSP>,
        regex_start_config: &regex_automata::util::start::Config,
        excepted_start_config: &regex_automata::util::start::Config,
        postdot_items: &AHashMap<Dotted<TI, TSP>, PostDotItems<TI, TD, TP, TSP, TS>>,
        to_be_completed_items_buffer: &mut AHashSet<ToBeCompletedItem<TI, TSP>>,
        deduplication_buffer: &mut AHashSet<EarleyItem<TI, TD, TP, TSP, TS>>,
        is_finished: &mut bool,
    ) {
        if let Some(PostDotItems::NormalItems(items)) = postdot_items.get(&Dotted {
            postdot_nonterminal_id: to_be_completed_item.nonterminal_id,
            column: to_be_completed_item.start_position,
        }) {
            for &item in items.iter() {
                Self::advance_item(
                    grammar,
                    to_be_completed_items_buffer,
                    regex_start_config,
                    excepted_start_config,
                    |item| {
                        deduplication_buffer.insert(item);
                    }, // Maybe we do not need to deduplicate in to_be_completed_items_buffer. Profiling is needed.
                    item,
                )
            }
        }
        if grammar.get_start_nonterminal_id() == to_be_completed_item.nonterminal_id
            && to_be_completed_item.start_position == TSP::ZERO
        {
            *is_finished = true;
        }
    }

    fn complete(
        grammar: &Grammar<TI, TE>,
        earley_sets: &mut EarleySets<TI, TD, TP, TSP, TS>,
        regex_start_config: &regex_automata::util::start::Config,
        excepted_start_config: &regex_automata::util::start::Config,
        to_be_completed_items: &mut AHashSet<ToBeCompletedItem<TI, TSP>>,
        to_be_completed_items_buffer: &mut AHashSet<ToBeCompletedItem<TI, TSP>>,
        leo_items: &mut AHashMap<ToBeCompletedItem<TI, TSP>, ToBeCompletedItem<TI, TSP>>,
        leo_items_buffer: &mut Vec<ToBeCompletedItem<TI, TSP>>,
        postdot_items: &AHashMap<Dotted<TI, TSP>, PostDotItems<TI, TD, TP, TSP, TS>>,
        deduplication_buffer: &mut AHashSet<EarleyItem<TI, TD, TP, TSP, TS>>,
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
                        topmost_item,
                        regex_start_config,
                        excepted_start_config,
                        postdot_items,
                        to_be_completed_items_buffer,
                        deduplication_buffer,
                        finished,
                    );
                } else {
                    Self::earley_complete_one_item(
                        grammar,
                        item,
                        regex_start_config,
                        excepted_start_config,
                        postdot_items,
                        to_be_completed_items_buffer,
                        deduplication_buffer,
                        finished,
                    );
                }
            }
            std::mem::swap(to_be_completed_items, to_be_completed_items_buffer);
        }
        for item in deduplication_buffer.drain() {
            earley_sets.push_to_last_row(item);
        }
    }

    fn revert_change(
        earley_sets: &mut EarleySets<TI, TD, TP, TSP, TS>,
        postdot_items: &mut AHashMap<Dotted<TI, TSP>, PostDotItems<TI, TD, TP, TSP, TS>>,
        added_postdot_items: &mut AHashSet<Dotted<TI, TSP>>,
        earley_set_length: usize,
        finished: &mut bool,
    ) {
        earley_sets.truncate::<0>(earley_set_length);
        *finished = false;
        for postdot in added_postdot_items.iter() {
            postdot_items.remove(postdot);
        }
        added_postdot_items.clear();
    }
    #[inline]
    fn commit_change(&mut self) {
        self.postdot_items_since_last_commit.clear();
    }
    #[inline]
    fn is_rejected(
        earley_sets: &EarleySets<TI, TD, TP, TSP, TS>,
        to_be_completed_items: &AHashSet<ToBeCompletedItem<TI, TSP>>,
    ) -> bool {
        earley_sets.view::<1, 1>([earley_sets.len() - 1]).is_empty()
            && to_be_completed_items.is_empty()
    }

    fn accept_byte(
        grammar: &Grammar<TI, TE>,
        earley_sets: &mut EarleySets<TI, TD, TP, TSP, TS>,
        to_be_completed_items: &mut AHashSet<ToBeCompletedItem<TI, TSP>>,
        to_be_completed_items_buffer: &mut AHashSet<ToBeCompletedItem<TI, TSP>>,
        leo_items: &mut AHashMap<ToBeCompletedItem<TI, TSP>, ToBeCompletedItem<TI, TSP>>,
        leo_items_buffer: &mut Vec<ToBeCompletedItem<TI, TSP>>,
        postdot_items: &mut AHashMap<Dotted<TI, TSP>, PostDotItems<TI, TD, TP, TSP, TS>>,
        added_postdot_items: &mut AHashSet<Dotted<TI, TSP>>,
        already_predicted_nonterminals: &mut FixedBitSet,
        deduplication_buffer: &mut AHashSet<EarleyItem<TI, TD, TP, TSP, TS>>,
        regex_start_config: &regex_automata::util::start::Config,
        excepted_start_config: &regex_automata::util::start::Config,
        previous_earley_set_length: usize,
        finished: &mut bool,
        byte: u8,
    ) -> Result<(), crate::engine_like::AcceptTokenError> {
        if *finished {
            Self::revert_change(
                earley_sets,
                postdot_items,
                added_postdot_items,
                previous_earley_set_length,
                finished,
            );
            return Err(crate::engine_like::AcceptTokenError::Rejected);
        }
        Self::scan(
            grammar,
            earley_sets,
            to_be_completed_items,
            regex_start_config,
            excepted_start_config,
            byte,
        ); // scan the current Earley set and creates the next Earley set
        if Self::is_rejected(earley_sets, to_be_completed_items) {
            Self::revert_change(
                earley_sets,
                postdot_items,
                added_postdot_items,
                previous_earley_set_length,
                finished,
            );
            return Err(crate::engine_like::AcceptTokenError::Rejected);
        }
        Self::complete(
            grammar,
            earley_sets,
            regex_start_config,
            excepted_start_config,
            to_be_completed_items,
            to_be_completed_items_buffer,
            leo_items,
            leo_items_buffer,
            postdot_items,
            deduplication_buffer,
            finished,
        ); // complete the next Earley set
        Self::predict(
            grammar,
            earley_sets,
            regex_start_config,
            excepted_start_config,
            already_predicted_nonterminals,
        ); // predict the next Earley set
        Self::update_postdot_items(grammar, earley_sets, postdot_items, added_postdot_items); // update postdot items for the next Earley set
        Ok(())
    }
}

#[allow(clippy::type_complexity)]
#[allow(clippy::too_many_arguments)]
impl<TI, TE, TD, TP, TSP, TS> EngineLike for EngineBase<TI, TE, TD, TP, TSP, TS>
where
    TI: Num
        + AsPrimitive<usize>
        + ConstOne
        + ConstZero
        + NumOps
        + NumAssign
        + std::cmp::PartialOrd
        + num::Bounded
        + std::convert::TryFrom<usize>
        + Debug,
    TI: Eq + std::hash::Hash + PartialEq,
    TE: AsPrimitive<usize>
        + ConstOne
        + ConstZero
        + Num
        + std::convert::TryFrom<usize>
        + num::Bounded
        + NumAssign,
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
    fn try_accept_new_token(
        &mut self,
        token_id: u32,
    ) -> Result<crate::engine_like::AcceptTokenResult, crate::engine_like::AcceptTokenError> {
        if self.is_finished() {
            return Err(crate::engine_like::AcceptTokenError::Finished);
        }
        let token = match self.vocabulary.get_token_from_token_id(token_id) {
            Some(token) => token,
            None => return Err(crate::engine_like::AcceptTokenError::UnknownTokenID),
        };
        let len = self.earley_sets.len();
        for byte in token.0.iter() {
            Self::accept_byte(
                &self.grammar,
                &mut self.earley_sets,
                &mut self.to_be_completed_items,
                &mut self.to_be_completed_items_buffer,
                &mut self.leo_items,
                &mut self.leo_items_buffer,
                &mut self.postdot_items,
                &mut self.postdot_items_since_last_commit,
                &mut self.already_predicted_nonterminals,
                &mut self.deduplication_buffer,
                &self.regex_start_config,
                &self.excepted_start_config,
                len,
                &mut self.finished,
                *byte,
            )?;
        }
        self.commit_change();
        if self.is_finished() {
            Ok(crate::engine_like::AcceptTokenResult::Finished)
        } else {
            Ok(crate::engine_like::AcceptTokenResult::Ongoing)
        }
    }

    fn compute_allowed_token_ids(&mut self) {
        self.allowed_token_ids.clear();
        if self.is_finished() {
            return;
        }
        let len = self.earley_sets.len();
        self.update_allowed_first_bytes();
        for byte in self.allowed_first_bytes.ones() {
            let mut current_token_id: Option<NonMaxU32> = None;
            let mut token_iter = self
                .vocabulary
                .get_normal_tokens_from_first_byte(byte as u8);
            #[allow(clippy::while_let_loop)]
            'outer: loop {
                if let Some(token_byte) = token_iter.next() {
                    match token_byte {
                        TokenIterItem::TokenByte(token_byte) => {
                            if Self::accept_byte(
                                &self.grammar,
                                &mut self.earley_sets,
                                &mut self.to_be_completed_items,
                                &mut self.to_be_completed_items_buffer,
                                &mut self.leo_items,
                                &mut self.leo_items_buffer,
                                &mut self.postdot_items,
                                &mut self.postdot_items_since_last_commit,
                                &mut self.already_predicted_nonterminals,
                                &mut self.deduplication_buffer,
                                &self.regex_start_config,
                                &self.excepted_start_config,
                                len,
                                &mut self.finished,
                                token_byte.into(),
                            )
                            .is_err()
                            // The token is rejected
                            {
                                loop {
                                    let a = token_iter.next();
                                    match a {
                                        Some(TokenIterItem::TokenByte(_)) => {} // skip the remaining token bytes
                                        Some(TokenIterItem::NewToken) => {
                                            // reach the next token
                                            current_token_id = token_iter.get_current_token_id();
                                            break;
                                        }
                                        None => {
                                            // reach the end of the token iterator
                                            break 'outer;
                                        }
                                    }
                                }
                            }
                        }
                        TokenIterItem::NewToken => {
                            // The token is accepted
                            Self::revert_change(
                                &mut self.earley_sets,
                                &mut self.postdot_items,
                                &mut self.postdot_items_since_last_commit,
                                len,
                                &mut self.finished,
                            );
                            if let Some(token_id) = current_token_id {
                                self.allowed_token_ids.insert(token_id.get() as usize);
                            }
                            current_token_id = token_iter.get_current_token_id();
                        }
                    }
                } else {
                    // reach the end of the token iterator, revert the last token's change
                    Self::revert_change(
                        &mut self.earley_sets,
                        &mut self.postdot_items,
                        &mut self.postdot_items_since_last_commit,
                        len,
                        &mut self.finished,
                    );
                    break;
                }
            }
        }
        for (token_id, token) in self.vocabulary.get_tokens_containing_separators() {
            let mut accepted = true;
            for byte in token.0.iter() {
                if Self::accept_byte(
                    &self.grammar,
                    &mut self.earley_sets,
                    &mut self.to_be_completed_items,
                    &mut self.to_be_completed_items_buffer,
                    &mut self.leo_items,
                    &mut self.leo_items_buffer,
                    &mut self.postdot_items,
                    &mut self.postdot_items_since_last_commit,
                    &mut self.already_predicted_nonterminals,
                    &mut self.deduplication_buffer,
                    &self.regex_start_config,
                    &self.excepted_start_config,
                    len,
                    &mut self.finished,
                    *byte,
                )
                .is_err()
                // The token is rejected
                {
                    accepted = false;
                    break;
                }
            }
            if accepted {
                self.allowed_token_ids.insert(token_id as usize);
                Self::revert_change(
                    &mut self.earley_sets,
                    &mut self.postdot_items,
                    &mut self.postdot_items_since_last_commit,
                    len,
                    &mut self.finished,
                );
            }
        }
        self.commit_change();
    }

    fn mask_logits(&self, logits: &mut [f32]) -> Result<(), crate::engine_like::MaskLogitsError> {
        if logits.len() != self.vocabulary.get_vocab_size() {
            return Err(crate::engine_like::MaskLogitsError::InvalidLogitsLength);
        }
        for (token_id, logit) in logits.iter_mut().enumerate() {
            if !self.allowed_token_ids.contains(token_id) {
                *logit = f32::NEG_INFINITY;
            }
        }
        Ok(())
    }

    fn update_logits(
        &mut self,
        token_id: u32,
        logits: &mut [f32],
    ) -> Result<crate::engine_like::AcceptTokenResult, crate::engine_like::UpdateLogitsError> {
        self.try_accept_new_token(token_id).map_err(|e| match e {
            crate::engine_like::AcceptTokenError::Finished => {
                crate::engine_like::UpdateLogitsError::Finished
            }
            crate::engine_like::AcceptTokenError::UnknownTokenID => {
                crate::engine_like::UpdateLogitsError::UnknownTokenID
            }
            crate::engine_like::AcceptTokenError::Rejected => {
                crate::engine_like::UpdateLogitsError::Rejected
            }
        })?;
        self.compute_allowed_token_ids();
        self.mask_logits(logits).map_err(|e| match e {
            crate::engine_like::MaskLogitsError::InvalidLogitsLength => {
                crate::engine_like::UpdateLogitsError::InvalidLogitsLength
            }
        })?;
        Ok(crate::engine_like::AcceptTokenResult::Ongoing)
    }

    fn get_allowed_token_ids_from_last_computation(&self) -> &FixedBitSet {
        &self.allowed_token_ids
    }

    fn is_finished(&self) -> bool {
        self.finished
    }

    fn reset(&mut self) {
        self.earley_sets.clear();
        self.to_be_completed_items.clear();
        self.to_be_completed_items_buffer.clear();
        self.leo_items.clear();
        self.leo_items_buffer.clear();
        self.postdot_items.clear();
        self.postdot_items_since_last_commit.clear();
        self.already_predicted_nonterminals.clear();
        self.finished = false;
        self.allowed_token_ids.clear();
        self.allowed_first_bytes.clear();
        self.earley_sets.new_row::<0>();
        Self::predict_nonterminal(
            &self.grammar,
            &mut self.earley_sets,
            &mut self.already_predicted_nonterminals,
            &self.regex_start_config,
            &self.excepted_start_config,
            self.grammar.get_start_nonterminal_id(),
            0,
        ); // init the first Earley set
        Self::predict(
            &self.grammar,
            &mut self.earley_sets,
            &self.regex_start_config,
            &self.excepted_start_config,
            &mut self.already_predicted_nonterminals,
        ); // run a full prediction for the first earley set
        Self::update_postdot_items(
            &self.grammar,
            &mut self.earley_sets,
            &mut self.postdot_items,
            &mut AHashSet::default(), // We will never need to revert the engine's state since it is the initialization
        );
    }

    fn into_boxed_engine(self) -> Box<dyn EngineLike> {
        Box::new(self)
    }
    fn get_vocab(&self) -> Arc<Vocabulary> {
        self.vocabulary.clone()
    }
}
