// Copyright 2017 The Chromium OS Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE file.

use std::mem;
use std::ops::Deref;
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd, RawFd};
use std::ptr;

use libc::{c_void, dup, eventfd, read, write};

use crate::{
    errno_result, AsRawDescriptor, FromRawDescriptor, IntoRawDescriptor, RawDescriptor, Result,
    SafeDescriptor,
};

/// A safe wrapper around a Linux eventfd (man 2 eventfd).
///
/// An eventfd is useful because it is sendable across processes and can be used for signaling in
/// and out of the KVM API. They can also be polled like any other file descriptor.
#[derive(Debug, PartialEq, Eq)]
pub struct EventFd {
    eventfd: SafeDescriptor,
}

impl EventFd {
    /// Creates a new blocking EventFd with an initial value of 0.
    pub fn new() -> Result<EventFd> {
        // This is safe because eventfd merely allocated an eventfd for our process and we handle
        // the error case.
        let ret = unsafe { eventfd(0, 0) };
        if ret < 0 {
            return errno_result();
        }
        // This is safe because we checked ret for success and know the kernel gave us an fd that we
        // own.
        Ok(EventFd {
            eventfd: unsafe { SafeDescriptor::from_raw_descriptor(ret) },
        })
    }

    /// Adds `v` to the eventfd's count, blocking until this won't overflow the count.
    pub fn write(&self, v: u64) -> Result<()> {
        // This is safe because we made this fd and the pointer we pass can not overflow because we
        // give the syscall's size parameter properly.
        let ret = unsafe {
            write(
                self.as_raw_fd(),
                &v as *const u64 as *const c_void,
                mem::size_of::<u64>(),
            )
        };
        if ret <= 0 {
            return errno_result();
        }
        Ok(())
    }

    /// Blocks until the the eventfd's count is non-zero, then resets the count to zero.
    pub fn read(&self) -> Result<u64> {
        let mut buf: u64 = 0;
        let ret = unsafe {
            // This is safe because we made this fd and the pointer we pass can not overflow because
            // we give the syscall's size parameter properly.
            read(
                self.as_raw_fd(),
                &mut buf as *mut u64 as *mut c_void,
                mem::size_of::<u64>(),
            )
        };
        if ret <= 0 {
            return errno_result();
        }
        Ok(buf)
    }

    /// Clones this EventFd, internally creating a new file descriptor. The new EventFd will share
    /// the same underlying count within the kernel.
    pub fn try_clone(&self) -> Result<EventFd> {
        // This is safe because we made this fd and properly check that it returns without error.
        let ret = unsafe { dup(self.as_raw_descriptor()) };
        if ret < 0 {
            return errno_result();
        }
        // This is safe because we checked ret for success and know the kernel gave us an fd that we
        // own.
        Ok(EventFd {
            eventfd: unsafe { SafeDescriptor::from_raw_descriptor(ret) },
        })
    }
}

impl AsRawFd for EventFd {
    fn as_raw_fd(&self) -> RawFd {
        self.eventfd.as_raw_fd()
    }
}

impl AsRawDescriptor for EventFd {
    fn as_raw_descriptor(&self) -> RawDescriptor {
        self.eventfd.as_raw_descriptor()
    }
}

impl FromRawFd for EventFd {
    unsafe fn from_raw_fd(fd: RawFd) -> Self {
        EventFd {
            eventfd: SafeDescriptor::from_raw_descriptor(fd),
        }
    }
}

impl IntoRawFd for EventFd {
    fn into_raw_fd(self) -> RawFd {
        self.eventfd.into_raw_descriptor()
    }
}

/// An `EventFd` wrapper which triggers when it goes out of scope.
///
/// If the underlying `EventFd` fails to trigger during drop, a panic is triggered instead.
pub struct ScopedEvent(EventFd);

impl ScopedEvent {
    /// Creates a new `ScopedEvent` which triggers when it goes out of scope.
    pub fn new() -> Result<ScopedEvent> {
        Ok(EventFd::new()?.into())
    }
}

impl From<EventFd> for ScopedEvent {
    fn from(e: EventFd) -> Self {
        Self(e)
    }
}

impl From<ScopedEvent> for EventFd {
    fn from(scoped_event: ScopedEvent) -> Self {
        // Rust doesn't allow moving out of types with a Drop implementation, so we have to use
        // something that copies instead of moves. This is safe because we prevent the drop of
        // `scoped_event` using `mem::forget`, so the underlying `EventFd` will not experience a
        // double-drop.
        let evt = unsafe { ptr::read(&scoped_event.0) };
        mem::forget(scoped_event);
        evt
    }
}

impl Deref for ScopedEvent {
    type Target = EventFd;

    fn deref(&self) -> &EventFd {
        &self.0
    }
}

impl Drop for ScopedEvent {
    fn drop(&mut self) {
        self.write(1).expect("failed to trigger scoped event");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new() {
        EventFd::new().unwrap();
    }

    #[test]
    fn read_write() {
        let evt = EventFd::new().unwrap();
        evt.write(55).unwrap();
        assert_eq!(evt.read(), Ok(55));
    }

    #[test]
    fn clone() {
        let evt = EventFd::new().unwrap();
        let evt_clone = evt.try_clone().unwrap();
        evt.write(923).unwrap();
        assert_eq!(evt_clone.read(), Ok(923));
    }

    #[test]
    fn scoped_event() {
        let scoped_evt = ScopedEvent::new().unwrap();
        let evt_clone: EventFd = scoped_evt.try_clone().unwrap();
        drop(scoped_evt);
        assert_eq!(evt_clone.read(), Ok(1));
    }

    #[test]
    fn eventfd_from_scoped_event() {
        let scoped_evt = ScopedEvent::new().unwrap();
        let evt: EventFd = scoped_evt.into();
        evt.write(1).unwrap();
    }
}
