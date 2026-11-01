//! A linked design (`links` in a design) may keep `source`: the SHA-256 of the part's design file
//! as it was last read, so SolidCraft can tell that the part's file changed since and update it
//! (docs/parts-library.md, "Changed parts"). An older design's links have none and need no rewrite:
//! they are checked once their part is next read. The last release refuses a design saved with
//! this (the newer format number).
