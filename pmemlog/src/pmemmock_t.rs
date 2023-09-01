use builtin::*;
use builtin_macros::*;
use crate::pmemspec_t::*;
use std::convert::*;
use vstd::prelude::*;

verus! {

    pub struct VolatileMemoryMockingPersistentMemory
    {
        contents: Vec<u8>
    }

    impl VolatileMemoryMockingPersistentMemory {
        #[verifier::external_body]
        pub fn new(capacity: u64) -> (result: Result<Self, ()>)
            ensures
                match result {
                    Ok(pm) => pm@.len() == capacity && pm.inv(),
                    Err(_) => true
                }
        {
            Ok(Self {contents: vec![0; capacity as usize]})
        }
    }

    impl PersistentMemory for VolatileMemoryMockingPersistentMemory {
        closed spec fn view(self) -> Seq<u8>
        {
            self.contents@
        }

        closed spec fn inv(self) -> bool
        {
            self.contents.len() <= u64::MAX
        }

        closed spec fn impervious_to_corruption(self) -> bool
        {
            true
        }

        #[verifier::external_body]
        fn get_capacity(&self) -> (result: u64)
        {
            self.contents.len().try_into().unwrap()
        }

        #[verifier::external_body]
        fn read(&self, addr: u64, num_bytes: u64) -> Vec<u8>
        {
            let addr_usize: usize = addr.try_into().unwrap();
            let num_bytes_usize: usize = num_bytes.try_into().unwrap();
            self.contents[addr_usize..addr_usize+num_bytes_usize].to_vec()
        }

        #[verifier::external_body]
        fn write(&mut self, addr: u64, bytes: &[u8])
        {
            let addr_usize: usize = addr.try_into().unwrap();
            self.contents.splice(addr_usize..addr_usize+bytes.len(), bytes.iter().cloned());
        }
    }

}
