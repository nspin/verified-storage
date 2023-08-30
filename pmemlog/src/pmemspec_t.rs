/*

  This file specifies, with the `PersistentMemory` type, the behavior
  of a persistent memory region. One of the things it models is what
  can happen to the persistent memory region if the system crashes in
  the middle of a write.

*/

use builtin::*;
use builtin_macros::*;
use vstd::prelude::*;
use vstd::bytes::*;
use vstd::set::*;
use vstd::slice::*;
use crate::sccf::CheckPermission;

#[cfg(not(verus_keep_ghost))]
use crc::Crc;

verus! {

    pub open spec fn maybe_corrupted(bytes: Seq<u8>, true_bytes: Seq<u8>, addrs: Seq<int>) -> bool {
        if bytes.len() != true_bytes.len() || bytes.len() != addrs.len() {
            false
        } else {
            forall |i: int| #![auto] 0 <= i < bytes.len() ==> maybe_corrupted_byte(bytes[i], true_bytes[i], addrs[i])
        }
    }

    pub closed spec fn maybe_corrupted_byte(byte: u8, true_byte: u8, addr: int) -> bool;

    pub open spec fn all_elements_unique(seq: Seq<int>) -> bool {
        forall |i: int, j: int| 0 <= i < j < seq.len() ==> seq[i] != seq[j]
    }

    pub const crc_size: u64 = 8; 

    pub closed spec fn spec_crc_bytes(header_bytes: Seq<u8>) -> Seq<u8>;

    #[verifier::external_body]
    pub exec fn bytes_crc(header_bytes: &Vec<u8>) -> (out: Vec<u8>)
        ensures
            spec_crc_bytes(header_bytes@) == out@,
            out@.len() == crc_size
    {
        #[cfg(not(verus_keep_ghost))]
        {
            let crc = Crc::<u64>::new(&crc::CRC_64_MS);
            let mut digest = crc.digest();
            digest.update(header_bytes);
            u64_to_le_bytes(digest.finalize())
        }
        #[cfg(verus_keep_ghost)]
        unimplemented!()
    }

    // We make two assumptions about how CRCs can be used to detect
    // corruption.

    // The first assumption, encapsulated in
    // `axiom_bytes_uncorrupted`, is that if we store byte sequences
    // `x` and `y` to persistent memory where `y` is the CRC of `x`,
    // then we can detect an absence of corruption by reading both of
    // them. Specifically, if we read from those locations and get
    // `x_c` and `y_c` (corruptions of `x` and `y` respectively), and
    // `y_c` is the CRC of `x_c`, then we can conclude that `x` wasn't
    // corrupted, i.e., that `x_c == x`.

    #[verifier(external_body)]
    pub proof fn axiom_bytes_uncorrupted(x_c: Seq<u8>, x: Seq<u8>, x_addrs: Seq<int>,
                                         y_c: Seq<u8>, y: Seq<u8>, y_addrs: Seq<int>)
        requires 
            maybe_corrupted(x_c, x, x_addrs),
            maybe_corrupted(y_c, y, y_addrs),
            y == spec_crc_bytes(x),
            y_c == spec_crc_bytes(x_c),
            all_elements_unique(x_addrs),
            all_elements_unique(y_addrs),
        ensures
            x == x_c
    {}

    // The second assumption, encapsulated in
    // `axiom_corruption_detecting_boolean`, is that the values
    // `cdb0_val` and `cdb1_val` are so randomly different from each
    // other that corruption can't make one appear to be the other.
    // That is, if we know we wrote either `cdb0_val` or `cdb1_val` to
    // a certain part of persistent memory, and when we read that same
    // part we get `cdb0_val` or `cdb1_val`, we can assume it matches
    // what we last wrote to it. To justify the assumption that
    // `cdb0_val` and `cdb1_val` are different from each other, we set
    // them to CRC(b"0") and CRC(b"1"), respectively.

    pub const cdb0_val: u64 = 0xa32842d19001605e; // CRC(b"0")
    pub const cdb1_val: u64 = 0xab21aa73069531b7; // CRC(b"1")

    #[verifier(external_body)]
    pub proof fn axiom_corruption_detecting_boolean(cdb_c: u64, cdb: u64, addrs: Seq<int>)
        requires 
            maybe_corrupted(spec_u64_to_le_bytes(cdb_c), spec_u64_to_le_bytes(cdb), addrs),
            all_elements_unique(addrs),
            cdb == cdb0_val || cdb == cdb1_val,
            cdb_c == cdb0_val || cdb_c == cdb1_val,
        ensures 
            cdb_c == cdb
    {}

    // We mark this as `external_body` so that the verifier can't see
    // that there's nothing important in it and thereby shortcut some
    // checks.

    pub trait PersistentMemory : Sized {
        spec fn view(self) -> Seq<u8>;

        spec fn inv(self) -> bool;

        fn new(capacity: u64) -> (result: Result<Self, ()>)
            ensures
                match result {
                    Ok(pm) => pm@.len() == capacity && pm.inv(),
                    Err(_) => true
                };

        spec fn impervious_to_corruption(self) -> bool;

        /// This is the model of some routine that queries how many
        /// bytes are in the persistent memory region.
        fn get_capacity(&self) -> (result: u64)
            requires
                self.inv()
            ensures
                result == self@.len();

        /// This is the model of some routine that reads the
        /// `num_bytes` bytes at address `addr`.
        fn read(&self, addr: u64, num_bytes: u64) -> (out: (Vec<u8>, Ghost<Seq<int>>))
            requires
                self.inv(),
                addr + num_bytes <= self@.len()
            ensures
                ({
                    let (bytes, addrs) = out;
                    &&& addrs == Ghost(Seq::<int>::new(num_bytes as nat, |i: int| i + addr))
                    &&& maybe_corrupted(bytes@, self@.subrange(addr as int, addr + num_bytes), addrs@)
                    &&& all_elements_unique(addrs@)
                    &&& self.impervious_to_corruption() ==> bytes@ == self@.subrange(addr as int, addr + num_bytes)
                });

        /// This is the model of some routine that writes `bytes`
        /// starting at address `addr`.
        fn write(&mut self, addr: u64, bytes: &[u8])
            requires
                old(self).inv(),
                addr + bytes@.len() <= (old(self))@.len(),
                addr + bytes@.len() <= u64::MAX
            ensures
                self.inv(),
                self@ == update_contents_to_reflect_write(old(self)@, addr as int, bytes@),
                self.impervious_to_corruption() == old(self).impervious_to_corruption();
    }

    /// We model the persistent memory as getting flushed in chunks,
    /// where each chunk has `persistence_chunk_size` bytes. We refer
    /// to chunk number `id` as the set of addresses `addr` such that
    /// `addr / persistence_chunk_size == id`.
    pub spec const persistence_chunk_size: int = 8;

    /// Return the byte at address `addr` after writing
    /// `write_bytes` to address `write_addr`, if the byte at
    /// `addr` before the write was `prewrite_byte`.
    pub open spec fn update_byte_to_reflect_write(addr: int, prewrite_byte: u8, write_addr: int,
                                                  write_bytes: Seq<u8>) -> u8
    {
        if write_addr <= addr && addr < write_addr + write_bytes.len() {
            write_bytes[addr - write_addr]
        }
        else {
            prewrite_byte
        }
    }

    /// Return the contents of persistent memory after writing
    /// `write_bytes` to address `write_addr`, if the contents
    /// before the write was `prewrite_contents`.
    pub open spec(checked) fn update_contents_to_reflect_write(prewrite_contents: Seq<u8>, write_addr: int,
                                                               write_bytes: Seq<u8>) -> Seq<u8>
        recommends
            0 <= write_addr,
            write_addr + write_bytes.len() <= prewrite_contents.len(),
    {
        Seq::<u8>::new(prewrite_contents.len(),
                       |addr| update_byte_to_reflect_write(addr, prewrite_contents[addr],
                                                           write_addr, write_bytes))
    }

    /// Return the byte at address `addr` after initiating (but
    /// not necessarily completing) a write of `write_bytes` to
    /// address `write_addr`, given that the byte at `addr` before
    /// the write was `prewrite_byte` and given that the set of
    /// chunk IDs that have been flushed since the initiation of
    /// the write is `chunks_flushed`.
    pub open spec fn update_byte_to_reflect_partially_flushed_write(addr: int, prewrite_byte: u8, write_addr: int,
                                                                    write_bytes: Seq<u8>,
                                                                    chunks_flushed: Set<int>) -> u8
    {
        if chunks_flushed.contains(addr / persistence_chunk_size) {
            update_byte_to_reflect_write(addr, prewrite_byte, write_addr, write_bytes)
        }
        else {
            prewrite_byte
        }
    }

    /// Return the contents of persistent memory after initiating
    /// (but not necessarily completing) a write of `write_bytes`
    /// to address `write_addr`, given that the contents before
    /// the write were `prewrite_contents` and given that the set of
    /// chunk IDs that have been flushed since the initiation of
    /// the write is `chunks_flushed`.
    pub open spec(checked) fn update_contents_to_reflect_partially_flushed_write(contents: Seq<u8>, write_addr: int,
                                                                                 write_bytes: Seq<u8>,
                                                                                 chunks_flushed: Set<int>) -> Seq<u8>
        recommends
            0 <= write_addr,
            write_addr + write_bytes.len() <= contents.len(),
    {
        Seq::<u8>::new(contents.len(),
                       |addr| update_byte_to_reflect_partially_flushed_write(addr, contents[addr], write_addr,
                                                                             write_bytes, chunks_flushed))
    }

    /// A `WriteRestrictedPersistentMemory<P>` object wraps a
    /// `PersistentMemory` object to restrict how it's written.
    /// Untrusted code passed one of these can only write to the
    /// encapsulated persistent memory by providing a permission of
    /// type `P`. That permission must allow all possible states `s`
    /// such that crashing in the middle of the write might leave the
    /// persistent memory in state `s`.
    pub struct WriteRestrictedPersistentMemory<Perm, PM>
        where
            Perm: CheckPermission<Seq<u8>>,
            PM: PersistentMemory
    {
        pm: PM,
        ghost perm: Option<Perm> // unused, but Rust demands some reference to Perm
    }

    impl <Perm, PM> WriteRestrictedPersistentMemory<Perm, PM>
        where
            Perm: CheckPermission<Seq<u8>>,
            PM: PersistentMemory
    {
        pub closed spec fn view(self) -> Seq<u8> {
            self.pm@
        }

        pub closed spec fn inv(self) -> bool {
            self.pm.inv()
        }

        pub closed spec fn impervious_to_corruption(self) -> bool {
            self.pm.impervious_to_corruption()
        }

        pub exec fn new(pm: PM) -> (wrpm: Self)
            requires
                pm.inv()
            ensures
                wrpm@ == pm@,
                wrpm.inv(),
                wrpm.impervious_to_corruption() == pm.impervious_to_corruption()
        {
            Self { pm: pm, perm: None }
        }

        pub exec fn get_pm_ref(&self) -> (pm: &PM)
            requires
                self.inv()
            ensures
                pm.inv(),
                pm@ == self@,
                pm.impervious_to_corruption() == self.impervious_to_corruption()
        {
            &self.pm
        }

        /// This `write` function can only be called if a crash in the
        /// middle of the requested write will leave the persistent
        /// memory in a state allowed by `perm`. The state must be
        /// allowed no matter what subset of the persistence chunks
        /// have been flushed.
        pub exec fn write(&mut self, addr: u64, bytes: &[u8], perm: Tracked<&Perm>)
            requires
                old(self).inv(),
                addr + bytes@.len() <= old(self)@.len(),
                addr + bytes@.len() <= u64::MAX,
                forall |chunks_flushed| {
                    let new_contents: Seq<u8> =
                        #[trigger] update_contents_to_reflect_partially_flushed_write(
                            old(self)@, addr as int, bytes@, chunks_flushed
                        );
                    perm@.check_permission(new_contents)
                },
            ensures
                self.inv(),
                self@ == update_contents_to_reflect_write(old(self)@, addr as int, bytes@),
                self.impervious_to_corruption() == old(self).impervious_to_corruption()
        {
            self.pm.write(addr, bytes)
        }
    }

}