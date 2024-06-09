use std::{fs::File, io::BufReader, path::Path, time::Duration};

use ahash::AHashMap;
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use kbnf::{
    engine::Engine,
    engine_like::EngineLike,
    vocabulary::{Token, Vocabulary},
};
#[derive(Debug, thiserror::Error)]
/// Error type when reading RWKV world model's vocabulary file.
pub enum ReadRWKVVocabError {
    #[error("IO error: {0}")]
    /// Error due to I/O operations like [Read], [Write], [Seek],
    IoError(#[from] std::io::Error),
    #[error("Serde json error: {0}")]
    JsonError(#[from] serde_json::Error),
}
/// Read the vocabulary from RWKV-world model series vocabulary file.
pub fn read_rwkv_world_vocab(path: impl AsRef<Path>) -> Result<Vocabulary, ReadRWKVVocabError> {
    let path = path.as_ref();
    let file = File::open(path).unwrap();
    let reader = BufReader::new(file);
    let mut id_to_token: AHashMap<u32, Token> = AHashMap::default();
    let mut id_to_token_string: AHashMap<u32, String> = AHashMap::default();
    let data: serde_json::Map<String, serde_json::Value> = serde_json::from_reader(reader)?;
    for (key, value) in data {
        let key = key.parse::<u32>().unwrap();
        match value {
            serde_json::Value::Array(x) => {
                let mut token = Vec::new();
                for x in x {
                    match x {
                        serde_json::Value::Number(x) => {
                            token.push(x.as_u64().unwrap() as u8);
                        }
                        _ => {
                            panic!("Unexpected value type")
                        }
                    }
                }
                id_to_token.insert(key, Token(token.clone().into_boxed_slice()));
                id_to_token_string.insert(key, format!("{:?}", token));
            }
            serde_json::Value::String(x) => {
                id_to_token.insert(key, Token(x.as_bytes().to_vec().into_boxed_slice()));
                id_to_token_string.insert(key, x);
            }
            _ => {
                panic!("Unexpected value type")
            }
        };
    }
    Ok(Vocabulary::new(id_to_token, id_to_token_string).unwrap())
}

fn run_an_engine(engine: &mut Engine, iteration: usize, token_id: u32) {
    for _ in 0..iteration {
        let _ = engine.try_accept_new_token(token_id).unwrap();
        engine.compute_allowed_token_ids();
    }
    engine.reset(); // reset the engine to its initial state while not deallocate memory
}

fn criterion_benchmark(c: &mut Criterion) {
    let mut c = c.benchmark_group("Simple");
    c.measurement_time(Duration::from_secs(10)).sample_size(100);
    let vocab = read_rwkv_world_vocab("tests/rwkv_vocab_v20230424.json").unwrap();
    let mut engine = Engine::new("start::=('{'start'}')?;", vocab.clone()).unwrap();
    c.bench_function("unmarked middle recursion 100 iterations", |b| {
        b.iter(|| run_an_engine(black_box(&mut engine), 100, 124))
    });
    let mut engine = Engine::new("start::=#\".+\"'\n';", vocab.clone()).unwrap();
    c.bench_function("always match regex 3 iterations", |b| {
        b.iter(|| run_an_engine(black_box(&mut engine), 3, 113))
    });
    let config = kbnf::config::Config {
        expected_output_length: 100,
        ..Default::default()
    };
    let mut engine = Engine::from_config("start::=#\".+\"'\n';", vocab.clone(), config).unwrap();
    c.bench_function(
        "always match regex 3 iterations (8 byte Earley item)",
        |b| b.iter(|| run_an_engine(black_box(&mut engine), 3, 113)),
    );
    let mut engine = Engine::new("start::=except!('\n\n')'\n\n';", vocab.clone()).unwrap();
    c.bench_function("simple except! 3 iterations", |b| {
        b.iter(|| run_an_engine(black_box(&mut engine), 3, 113))
    });
    let mut engine = Engine::new("start::=except!('\n\n',5)'\n\n';", vocab.clone()).unwrap();
    c.bench_function("simple except! with repetition 5 3 iterations", |b| {
        b.iter(|| run_an_engine(black_box(&mut engine), 3, 113))
    });
    let mut engine = Engine::new("start::=except!('\n\n',50)'\n\n';", vocab.clone()).unwrap();
    c.bench_function("simple except! with repetition 50 3 iterations", |b| {
        b.iter(|| run_an_engine(black_box(&mut engine), 3, 113))
    });
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
