//! An internal thread can make its hole the size's tap drill first: Thread features save
//! `fit_hole: true` (not saved when false; `bore`, its name in a development build, reads as it).
//! Over the threaded length a narrower hole is bored out and a wider one filled in, so any size
//! can be picked for a hole, modelled or cosmetic, and the feature says what it changed. Older
//! designs have no `fit_hole` and leave their holes as they were (a modelled thread refusing one
//! its teeth do not fit), so they open as they were. The last release refuses a design with this
//! change (the newer format number).
