//! A thread can start from the other end of its face: Thread features save `flip: true` (not
//! saved when false), and the end the thread starts from (its limited length, runout and
//! chamfered tip) is the other one from what `from`, or the face, would choose. Older designs have
//! no `flip` and start where they did. The last release refuses a design with this change (the
//! newer format number).
