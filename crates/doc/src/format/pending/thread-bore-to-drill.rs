//! A modelled thread can bore its hole to the size's tap drill first: Thread features save
//! `bore: true` (not saved when false). A hole too narrow for the thread's teeth is then widened
//! to the tap drill over the threaded length before the groove is cut, as a drill would before
//! the tap, and the feature says so. Older designs have no `bore` and keep refusing such a hole
//! with the message they gave, so they open as they were. The last release refuses a design
//! with this change (the newer format number).
