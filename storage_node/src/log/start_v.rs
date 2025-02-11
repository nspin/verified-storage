//! This file contains functions for starting to use persistent memory
//! as a log. Such starting is done either after setup or after a
//! crash.
//!
//! The code in this file is verified and untrusted (as indicated by
//! the `_v.rs` suffix), so you don't have to read it to be confident
//! of the system's correctness.

use crate::log::inv_v::*;
use crate::log::layout_v::*;
use crate::log::logimpl_t::LogErr;
use crate::log::logimpl_v::LogInfo;
use crate::log::logspec_t::AbstractLogState;
use crate::pmem::pmemspec_t::{PersistentMemoryRegion, CRC_SIZE};
use crate::pmem::pmemutil_v::{check_cdb, check_crc, check_crc_deserialized};
use crate::pmem::serialization_t::*;
use builtin::*;
use builtin_macros::*;
use vstd::arithmetic::div_mod::*;
use vstd::bytes::*;
use vstd::prelude::*;
use vstd::slice::*;

verus! {

    // This exported function reads the corruption-detecting boolean
    // and returns it.
    //
    // `pm_region` -- the persistent-memory region to read from
    //
    // The result is a `Result<bool, LogErr>` with the following meanings:
    //
    // `Err(LogErr::CRCMismatch)` -- The CDB couldn't be read due
    // to a CRC error.
    //
    // `Ok(b)` -- The CDB could be read and represents the boolean `b`.
    pub fn read_cdb<PMRegion: PersistentMemoryRegion>(pm_region: &PMRegion) -> (result: Result<bool, LogErr>)
        requires
            pm_region.inv(),
            recover_cdb(pm_region@.committed()).is_Some(),
            pm_region@.no_outstanding_writes(),
        ensures
            match result {
                Ok(b) => Some(b) == recover_cdb(pm_region@.committed()),
                // To make sure this code doesn't spuriously generate CRC-mismatch errors,
                // it's obligated to prove that it won't generate such an error when
                // the persistent memory is impervious to corruption.
                Err(LogErr::CRCMismatch) => !pm_region.constants().impervious_to_corruption,
                _ => false,
            }
    {
        let ghost mem = pm_region@.committed();

        // let log_cdb_bytes = pm_region.read(ABSOLUTE_POS_OF_LOG_CDB, CRC_SIZE);
        let log_cdb = pm_region.read_and_deserialize::<u64>(ABSOLUTE_POS_OF_LOG_CDB);
        let result = check_cdb(&log_cdb, Ghost(mem),
                               Ghost(pm_region.constants().impervious_to_corruption),
                               Ghost(ABSOLUTE_POS_OF_LOG_CDB));
        match result {
            Some(b) => Ok(b),
            None => Err(LogErr::CRCMismatch)
        }
    }

    // This function reads the log information for a single log from
    // persistent memory.
    //
    // `pm_region` -- the persistent memory region to read from
    //
    // `log_id` -- the GUID of the log
    //
    // `cdb` -- the corruption-detection boolean
    //
    // The result is a `Result<LogInfo, LogErr>` with the following meanings:
    //
    // `Ok(log_info)` -- The information `log_info` has been
    // successfully read.
    //
    // `Err(LogErr::CRCMismatch)` -- The region couldn't be read due
    // to a CRC error when reading data.
    //
    // `Err(LogErr::StartFailedDueToProgramVersionNumberUnsupported)`
    // -- The program version number stored in persistent memory is
    // one that this code doesn't know how to recover from. It was
    // presumably created by a later version of this code.
    //
    // `Err(LogErr::StartFailedDueToLogIDMismatch)` -- The
    // log ID stored in persistent memory doesn't match the one
    // passed to the `start` routine. So the caller of `start` gave
    // the wrong persistent memory region or the wrong ID.
    //
    // `Err(LogErr::StartFailedDueToRegionSizeMismatch)` -- The
    // region size stored in persistent memory doesn't match the size
    // of the region passed to the `start` routine. So the caller of
    // `start` is likely using a persistent memory region that starts
    // in the right place but ends in the wrong place.
    //
    // `Err(LogErr::StartFailedDueToInvalidMemoryContents)` --
    // The region's contents aren't valid, i.e., they're not
    // recoverable to a valid log. The user must have requested to
    // start using the wrong region of persistent memory.
    pub fn read_log_variables<PMRegion: PersistentMemoryRegion>(
        pm_region: &PMRegion,
        log_id: u128,
        cdb: bool,
    ) -> (result: Result<LogInfo, LogErr>)
        requires
            pm_region.inv(),
            pm_region@.no_outstanding_writes(),
        ensures
            ({
                let state = recover_given_cdb(pm_region@.committed(), log_id, cdb);
                match result {
                    Ok(info) => state.is_Some() ==> {
                        &&& metadata_consistent_with_info(pm_region@, log_id, cdb, info)
                        &&& info_consistent_with_log_area(pm_region@, info, state.unwrap())
                    },
                    Err(LogErr::CRCMismatch) =>
                        state.is_Some() ==> !pm_region.constants().impervious_to_corruption,
                    _ => state.is_None()
                }
            })
    {
        let ghost mem = pm_region@.committed();
        let ghost state = recover_given_cdb(pm_region@.committed(), log_id, cdb);

        // Check that the region is at least the minimum required size. If
        // not, indicate invalid memory contents.

        let region_size = pm_region.get_region_size();
        if region_size < ABSOLUTE_POS_OF_LOG_AREA + MIN_LOG_AREA_SIZE {
            assert(state.is_None()); // This can't happen if the persistent memory is recoverable
            return Err(LogErr::StartFailedDueToInvalidMemoryContents)
        }

        // Read the global metadata and its CRC, and check that the
        // CRC matches.

        let global_metadata = pm_region.read_and_deserialize::<GlobalMetadata>(ABSOLUTE_POS_OF_GLOBAL_METADATA);
        let global_crc = pm_region.read_and_deserialize(ABSOLUTE_POS_OF_GLOBAL_CRC);
        if !check_crc_deserialized(global_metadata, global_crc,
                      Ghost(mem), Ghost(pm_region.constants().impervious_to_corruption),
                      Ghost(ABSOLUTE_POS_OF_GLOBAL_METADATA), Ghost(LENGTH_OF_GLOBAL_METADATA),
                      Ghost(ABSOLUTE_POS_OF_GLOBAL_CRC)) {
            return Err(LogErr::CRCMismatch);
        }

        // Check the global metadata for validity. If it isn't valid,
        // e.g., due to the program GUID not matching, then return an
        // error. Such invalidity can't happen if the persistent
        // memory is recoverable.

        if global_metadata.program_guid != LOG_PROGRAM_GUID {
            assert(state.is_None()); // This can't happen if the persistent memory is recoverable
            return Err(LogErr::StartFailedDueToInvalidMemoryContents)
        }

        if global_metadata.version_number != LOG_PROGRAM_VERSION_NUMBER {
            assert(state.is_None()); // This can't happen if the persistent memory is recoverable
            return Err(LogErr::StartFailedDueToProgramVersionNumberUnsupported{
                version_number: global_metadata.version_number,
                max_supported: LOG_PROGRAM_VERSION_NUMBER,
            })
        }

        if global_metadata.length_of_region_metadata != LENGTH_OF_REGION_METADATA {
            assert(state.is_None()); // This can't happen if the persistent memory is recoverable
            return Err(LogErr::StartFailedDueToInvalidMemoryContents)
        }

        // Read the region metadata and its CRC, and check that the
        // CRC matches.

        let region_metadata = pm_region.read_and_deserialize::<RegionMetadata>(ABSOLUTE_POS_OF_REGION_METADATA);
        let region_crc = pm_region.read_and_deserialize(ABSOLUTE_POS_OF_REGION_CRC);
        if !check_crc_deserialized(region_metadata, region_crc,
                      Ghost(mem), Ghost(pm_region.constants().impervious_to_corruption),
                      Ghost(ABSOLUTE_POS_OF_REGION_METADATA), Ghost(LENGTH_OF_REGION_METADATA),
                      Ghost(ABSOLUTE_POS_OF_REGION_CRC)) {
            return Err(LogErr::CRCMismatch);
        }

        // Check the region metadata for validity. If it isn't valid,
        // e.g., due to the encoded region size not matching the
        // actual region size, then return an error. Such invalidity
        // can't happen if the persistent memory is recoverable.

        if region_metadata.region_size != region_size {
            assert(state.is_None()); // This can't happen if the persistent memory is recoverable
            return Err(LogErr::StartFailedDueToRegionSizeMismatch{
                region_size_expected: region_size,
                region_size_read: region_metadata.region_size,
            })
        }

        if region_metadata.log_id != log_id {
            assert(state.is_None()); // This can't happen if the persistent memory is recoverable
            return Err(LogErr::StartFailedDueToLogIDMismatch{
                log_id_expected: log_id,
                log_id_read: region_metadata.log_id,
            })
        }

        if region_metadata.log_area_len > region_size {
            assert(state.is_None()); // This can't happen if the persistent memory is recoverable
            return Err(LogErr::StartFailedDueToInvalidMemoryContents)
        }
        if region_size - region_metadata.log_area_len < ABSOLUTE_POS_OF_LOG_AREA {
            assert(state.is_None()); // This can't happen if the persistent memory is recoverable
            return Err(LogErr::StartFailedDueToInvalidMemoryContents)
        }
        if region_metadata.log_area_len < MIN_LOG_AREA_SIZE {
            assert(state.is_None()); // This can't happen if the persistent memory is recoverable
            return Err(LogErr::StartFailedDueToInvalidMemoryContents)
        }

        // Read the log metadata and its CRC, and check that the
        // CRC matches. The position where to find the log
        // metadata depend on the CDB.

        let log_metadata_pos = if cdb { ABSOLUTE_POS_OF_LOG_METADATA_FOR_CDB_TRUE }
                                  else { ABSOLUTE_POS_OF_LOG_METADATA_FOR_CDB_FALSE };
        let log_crc_pos = if cdb { ABSOLUTE_POS_OF_LOG_CRC_FOR_CDB_TRUE }
                             else { ABSOLUTE_POS_OF_LOG_CRC_FOR_CDB_FALSE };
        let log_metadata = pm_region.read_and_deserialize::<LogMetadata>(log_metadata_pos);
        let log_crc = pm_region.read_and_deserialize::<u64>(log_crc_pos);
        if !check_crc_deserialized(log_metadata, log_crc, Ghost(mem),
                                   Ghost(pm_region.constants().impervious_to_corruption),
                                    Ghost(log_metadata_pos), Ghost(LENGTH_OF_LOG_METADATA), Ghost(log_crc_pos)) {
            return Err(LogErr::CRCMismatch);
        }

        // Check the log metadata for validity. If it isn't valid,
        // e.g., due to the log length being greater than the log area
        // length, then return an error. Such invalidity can't happen
        // if the persistent memory is recoverable.

        let head = log_metadata.head;
        let log_length = log_metadata.log_length;
        if log_length > region_metadata.log_area_len {
            assert(state.is_None()); // This can't happen if the persistent memory is recoverable
            return Err(LogErr::StartFailedDueToInvalidMemoryContents)
        }
        if log_length as u128 > u128::MAX - head {
            assert(state.is_None()); // This can't happen if the persistent memory is recoverable
            return Err(LogErr::StartFailedDueToInvalidMemoryContents)
        }

        // Compute the offset into the log area where the head of the
        // log is. This is the u128 `head` mod the u64
        // `log_area_len`. To prove that this will fit in a `u64`, we
        // need to invoke a math lemma saying that the result of a
        // modulo operation is always less than the divisor.

        proof { lemma_mod_bound(head as int, region_metadata.log_area_len as int); }
        let head_log_area_offset: u64 = (head % region_metadata.log_area_len as u128) as u64;

        // Return the log info. This necessitates computing the
        // pending tail position relative to the head, but this is
        // easy: It's the same as the log length. This is because,
        // upon recovery, there are no pending appends beyond the tail
        // of the log.

        Ok(LogInfo{
            log_area_len: region_metadata.log_area_len,
            head,
            head_log_area_offset,
            log_length,
            log_plus_pending_length: log_length
        })
    }
}
