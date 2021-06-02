/*
 * Created on Wed Jun 02 2021
 *
 * This file is a part of Skytable
 * Skytable (formerly known as TerrabaseDB or Skybase) is a free and open-source
 * NoSQL database written by Sayan Nandan ("the Author") with the
 * vision to provide flexibility in data modelling without compromising
 * on performance, queryability or scalability.
 *
 * Copyright (c) 2021, Sayan Nandan <ohsayan@outlook.com>
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU Affero General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU Affero General Public License for more details.
 *
 * You should have received a copy of the GNU Affero General Public License
 * along with this program. If not, see <https://www.gnu.org/licenses/>.
 *
*/

#![allow(dead_code)] // TODO(@ohsayan): Remove this lint once we're done

mod generic {
    //! Implementations for CPU architectures that do not support SSE instructions
    /*
        TODO(@ohsayan): Evaluate the need for NEON/AVX. Also note, SSE3/ SSE4 can
        prove to have much faster vector operations, but older CPUs may not support it.
        Our job is to first build for SSE2 since that has the best support (all the way from Pentium
        chips). NEON has multi-cycle latencies, so that needs more evaluation.

        Note about the `GroupWord`s: we choose the target's pointer word width than just blindly
        using 64-bit pointer sizes because using 64-bit on 32-bit systems would only add to higher
    */

    use super::control_bytes;
    use core::mem;
    use core::ptr;

    #[cfg(target_pointer_width = "64")]
    type GroupWord = u64;

    #[cfg(target_pointer_width = "32")]
    type GroupWord = u32;

    /// Just use the expected pointer width publicly for sanity
    pub type BitMaskWord = GroupWord;

    pub const BITMASK_STRIDE: usize = 8;
    pub const BITMASK_MASK: BitMaskWord = 0x8080_8080_8080_8080_u64 as BitMaskWord;

    /// A group of control-bytes that can be scanned in parallel
    pub struct Group(GroupWord);

    impl Group {
        /// This will return either 32/64 depending on the target's pointer width
        pub const WIDTH: usize = mem::size_of::<Self>();
        /// Returns a full group
        pub const fn empty_static() -> &'static [u8; Group::WIDTH] {
            #[repr(C)]
            struct AlignedBytes {
                // some explicit padding for alignment to ensure alignment to the group size
                _align: [Group; 0],
                bytes: [u8; Group::WIDTH],
            }
            #[allow(dead_code)] // Clippy doesn't know that we're getting aligned bytes here, so suppress this lint
            const ALIGNED_BYTES: AlignedBytes = AlignedBytes {
                _align: [],
                bytes: [control_bytes::EMPTY; Group::WIDTH],
            };
            &ALIGNED_BYTES.bytes
        }

        /// Load a group of bytes starting at the provided address (unaligned read)
        pub unsafe fn load_unaligned(ptr: *const u8) -> Self {
            Group(ptr::read_unaligned(ptr.cast()))
        }

        /// Load a group of bytes starting at the provided address (aligned read)
        pub unsafe fn load_aligned(ptr: *const u8) -> Self {
            Group(ptr::read(ptr.cast()))
        }

        /// Store the [`Group`] in the given address. This is guaranteed to be aligned
        pub unsafe fn store_aligned(self, ptr: *mut u8) {
            ptr::write(ptr.cast(), self.0)
        }
    }
}

mod mapalloc {
    //! Primitive methods for allocation
    use core::alloc::Layout;
    use core::ptr::NonNull;
    use std::alloc;

    /// This trait defines an allocator. The reason we don't directly use the host allocator
    /// and abstract it away with a trait is for future events when we may build our own
    /// allocator (or maybe support embedded!? gosh, that'll be some task)
    pub unsafe trait Allocator {
        fn allocate(&self, layout: Layout) -> Result<NonNull<u8>, ()> {
            unsafe { NonNull::new(alloc::alloc(layout)).ok_or(()) }
        }
        unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
            alloc::dealloc(ptr.as_ptr(), layout)
        }
    }

    pub struct Global;
    impl Default for Global {
        fn default() -> Self {
            Global
        }
    }

    /// Use a given allocator `A` to allocate for a given memory layout
    pub fn self_allocate<A: Allocator>(allocator: &A, layout: Layout) -> Result<NonNull<u8>, ()> {
        allocator.allocate(layout)
    }
}

mod control_bytes {
    /// Control byte value for an empty bucket.
    pub const EMPTY: u8 = 0b1111_1111;
}