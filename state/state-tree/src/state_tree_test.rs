use super::*;
use crate::mock::MockStateNodeStore;
use anyhow::Result;
use forkable_jellyfish_merkle::blob::Blob;
use forkable_jellyfish_merkle::{HashValueKey, RawKey};
use starcoin_config::RocksdbConfig;
use starcoin_crypto::hash::*;
use starcoin_storage::db_storage::DBStorage;
use starcoin_storage::storage::StorageInstance;
use starcoin_storage::Storage;
use std::collections::HashMap;
use std::sync::Arc;

/// change the `n`th nibble to `nibble`
pub fn update_nibble(original_key: &HashValueKey, n: usize, nibble: u8) -> HashValueKey {
    assert!(nibble < 16);
    let mut key = original_key.key_hash().to_vec();
    key[n / 2] = if n % 2 == 0 {
        key[n / 2] & 0x0f | nibble << 4
    } else {
        key[n / 2] & 0xf0 | nibble
    };
    HashValueKey(HashValue::from_slice(&key).unwrap())
}

#[test]
pub fn test_put_blob() -> Result<()> {
    let s = MockStateNodeStore::new();
    let state = StateTree::<HashValueKey>::new(Arc::new(s), None);
    assert_eq!(state.root_hash(), *SPARSE_MERKLE_PLACEHOLDER_HASH);

    let hash_value = HashValue::random().into();

    let account1 = update_nibble(&hash_value, 0, 1);
    let account1 = update_nibble(&account1, 2, 2);
    state.put(account1, vec![0, 0, 0]);

    assert_eq!(state.get(&account1)?, Some(vec![0, 0, 0]));
    assert_eq!(state.get(&update_nibble(&hash_value, 0, 8))?, None);

    let new_root_hash = state.commit()?;
    assert_eq!(state.root_hash(), new_root_hash);
    assert_eq!(state.get(&account1)?, Some(vec![0, 0, 0]));
    assert_eq!(state.get(&update_nibble(&hash_value, 0, 8))?, None);

    let (root, updates) = state.change_sets();
    assert_eq!(root, new_root_hash);
    assert_eq!(updates.num_stale_leaves, 0);
    assert_eq!(updates.num_new_leaves, 1);
    assert_eq!(updates.node_batch.len(), 1);
    assert_eq!(updates.stale_node_index_batch.len(), 1);

    let account2 = update_nibble(&account1, 0, 2);
    state.put(account2, vec![0, 0, 0]);
    assert_eq!(state.get(&account2)?, Some(vec![0, 0, 0]));
    let new_root_hash = state.commit()?;
    assert_eq!(state.root_hash(), new_root_hash);
    assert_eq!(state.get(&account2)?, Some(vec![0, 0, 0]));
    let (root, updates) = state.change_sets();
    assert_eq!(root, new_root_hash);
    assert_eq!(updates.num_stale_leaves, 0);
    assert_eq!(updates.num_new_leaves, 2);
    assert_eq!(updates.node_batch.len(), 3);
    assert_eq!(updates.stale_node_index_batch.len(), 1);

    // modify existed account
    state.put(account1, vec![1, 1, 1]);
    assert_eq!(state.get(&account1)?, Some(vec![1, 1, 1]));
    let new_root_hash = state.commit()?;
    assert_eq!(state.root_hash(), new_root_hash);
    assert_eq!(state.get(&account1)?, Some(vec![1, 1, 1]));
    let (root, updates) = state.change_sets();
    assert_eq!(root, new_root_hash);
    assert_eq!(updates.num_stale_leaves, 0);
    assert_eq!(updates.num_new_leaves, 2);
    assert_eq!(updates.node_batch.len(), 3);
    assert_eq!(updates.stale_node_index_batch.len(), 1);

    let account3 = update_nibble(&account1, 2, 3);
    for (k, v) in vec![(account1, vec![1, 1, 0]), (account3, vec![0, 0, 0])] {
        state.put(k, v);
    }
    assert_eq!(state.get(&account1)?, Some(vec![1, 1, 0]));
    assert_eq!(state.get(&account2)?, Some(vec![0, 0, 0]));
    assert_eq!(state.get(&account3)?, Some(vec![0, 0, 0]));

    let new_root_hash = state.commit()?;
    assert_eq!(state.root_hash(), new_root_hash);
    assert_eq!(state.get(&account1)?, Some(vec![1, 1, 0]));
    assert_eq!(state.get(&account2)?, Some(vec![0, 0, 0]));
    assert_eq!(state.get(&account3)?, Some(vec![0, 0, 0]));

    let (_, updates) = state.change_sets();
    assert_eq!(updates.num_stale_leaves, 0);
    assert_eq!(updates.num_new_leaves, 3);
    assert_eq!(updates.node_batch.len(), 6);
    assert_eq!(updates.stale_node_index_batch.len(), 1);
    Ok(())
}

#[test]
pub fn test_state_proof() -> Result<()> {
    let s = MockStateNodeStore::new();
    let state = StateTree::new(Arc::new(s), None);
    assert_eq!(state.root_hash(), *SPARSE_MERKLE_PLACEHOLDER_HASH);

    let hash_value = HashValue::random().into();

    let account1 = update_nibble(&hash_value, 0, 1);
    // re-update to make sure account2 never equal to account1
    let account1 = update_nibble(&account1, 2, 1);

    let account2 = update_nibble(&account1, 2, 2);
    for (k, v) in vec![(account1, vec![0, 0, 0]), (account2, vec![1, 1, 1])] {
        state.put(k, v);
    }
    let (value, _) = state.get_with_proof(&account1)?;
    assert!(value.is_none());
    let new_root_hash = state.commit()?;
    let (value, proof) = state.get_with_proof(&account1)?;
    assert!(value.is_some());
    assert_eq!(value.unwrap(), vec![0, 0, 0]);
    let expected_value = Some(vec![0u8, 0, 0].into());
    proof.verify(new_root_hash, account1.key_hash(), expected_value.as_ref())?;

    state.remove(&account1);
    let new_root_hash = state.commit()?;
    let (value, proof) = state.get_with_proof(&account1)?;
    assert!(value.is_none());
    proof.verify(new_root_hash, account1.key_hash(), None)?;

    Ok(())
}

#[test]
pub fn test_state_commit() -> Result<()> {
    let s = MockStateNodeStore::new();
    let state = StateTree::new(Arc::new(s), None);
    assert_eq!(state.root_hash(), *SPARSE_MERKLE_PLACEHOLDER_HASH);

    let hash_value = HashValue::random().into();

    let account1 = update_nibble(&hash_value, 0, 1);
    let account1 = update_nibble(&account1, 2, 2);
    state.put(account1, vec![0, 0, 0]);
    let _new_root_hash = state.commit()?;

    let account3 = update_nibble(&account1, 2, 3);
    for (k, v) in vec![(account1, vec![1, 1, 0]), (account3, vec![0, 0, 0])] {
        state.put(k, v);
    }
    let new_root_hash = state.commit()?;

    state.flush()?;
    assert_eq!(state.root_hash(), new_root_hash);
    assert_eq!(state.get(&account1)?, Some(vec![1, 1, 0]));
    assert_eq!(state.get(&account3)?, Some(vec![0, 0, 0]));
    assert_eq!(state.get(&update_nibble(&account1, 2, 10))?, None);
    Ok(())
}

#[test]
pub fn test_state_dump() -> Result<()> {
    let s = MockStateNodeStore::new();
    let state = StateTree::new(Arc::new(s), None);
    let hash_value = HashValueKey(HashValue::random());
    let value = vec![1u8, 2u8];
    state.put(hash_value, value);
    state.commit()?;
    let state_set = state.dump()?;
    assert_eq!(1, state_set.len());
    Ok(())
}

#[test]
pub fn test_repeat_commit() -> Result<()> {
    let s = MockStateNodeStore::new();
    let state = StateTree::new(Arc::new(s), None);
    let hash_value = HashValueKey(HashValue::random());
    let value = vec![1u8, 2u8];
    state.put(hash_value, value.clone());
    state.commit()?;

    let root_hash1 = state.root_hash();
    state.put(hash_value, value);
    state.commit()?;
    let root_hash2 = state.root_hash();
    assert_eq!(root_hash1, root_hash2);
    Ok(())
}

#[test]
pub fn test_state_storage_dump() -> Result<()> {
    let storage = MockStateNodeStore::new();
    let state = StateTree::new(Arc::new(storage), None);
    let hash_value1 = HashValueKey(HashValue::random());
    let value1 = vec![1u8, 2u8];
    state.put(hash_value1, value1.clone());
    let hash_value2 = HashValueKey(HashValue::random());
    let value2 = vec![3u8, 4u8];
    state.put(hash_value2, value2.clone());
    state.commit()?;
    let state_set = state.dump()?;
    assert_eq!(2, state_set.len());
    let mut iter = state.dump_iter()?;
    let mut kv1 = HashMap::new();
    kv1.insert(hash_value1, Blob::from(value1));
    kv1.insert(hash_value2, Blob::from(value2));
    let mut kv2 = HashMap::new();
    let v1 = iter.next().unwrap()?;
    let v2 = iter.next().unwrap()?;
    assert!(iter.next().is_none(), "iter next should none");
    kv2.insert(v1.0, v1.1);
    kv2.insert(v2.0, v2.1);
    assert_eq!(kv1, kv2);
    Ok(())
}

#[test]
pub fn test_state_multi_commit_missing_node() -> Result<()> {
    let tmpdir = starcoin_config::temp_dir();
    let instance = StorageInstance::new_db_instance(DBStorage::new(
        tmpdir.path(),
        RocksdbConfig::default(),
        None,
    )?);
    let storage = Storage::new(instance)?;
    let state = StateTree::new(Arc::new(storage.clone()), None);
    let hash_value1 = HashValueKey(HashValue::random());
    let value1 = vec![1u8, 2u8];
    state.put(hash_value1, value1);
    state.commit()?;
    let root_hash1 = state.root_hash();
    let hash_value2 = HashValueKey(HashValue::random());
    let value12 = vec![12u8, 2u8];
    let value2 = vec![3u8, 4u8];
    state.put(hash_value1, value12.clone());
    state.put(hash_value2, value2.clone());
    state.commit()?;
    state.flush()?;
    let root_hash2 = state.root_hash();
    let state1 = StateTree::new(Arc::new(storage.clone()), Some(root_hash1));
    let result = state1.get(&hash_value1);
    assert!(result.is_err(), "Missing node at HashValue");

    let state2 = StateTree::new(Arc::new(storage), Some(root_hash2));
    assert_eq!(state2.get(&hash_value1)?, Some(value12));
    assert_eq!(state2.get(&hash_value2)?, Some(value2));
    Ok(())
}

#[test]
pub fn test_state_multi_commit_and_flush() -> Result<()> {
    let tmpdir = starcoin_config::temp_dir();
    let instance = StorageInstance::new_db_instance(DBStorage::new(
        tmpdir.path(),
        RocksdbConfig::default(),
        None,
    )?);
    let storage = Storage::new(instance)?;
    let state = StateTree::new(Arc::new(storage.clone()), None);
    let hash_value1 = HashValueKey(HashValue::random());
    let value1 = vec![1u8, 2u8];
    state.put(hash_value1, value1.clone());
    state.commit()?;
    state.flush()?;
    let root_hash1 = state.root_hash();
    let hash_value2 = HashValueKey(HashValue::random());
    let value12 = vec![12u8, 2u8];
    let value2 = vec![3u8, 4u8];
    state.put(hash_value1, value12.clone());
    state.put(hash_value2, value2.clone());
    state.commit()?;
    state.flush()?;
    let root_hash2 = state.root_hash();
    let state1 = StateTree::new(Arc::new(storage.clone()), Some(root_hash1));
    assert_eq!(state1.get(&hash_value1)?, Some(value1));

    let state2 = StateTree::new(Arc::new(storage), Some(root_hash2));
    assert_eq!(state2.get(&hash_value1)?, Some(value12));
    assert_eq!(state2.get(&hash_value2)?, Some(value2));
    Ok(())
}
