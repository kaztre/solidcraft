//! A modelled thread can leave clearance for a printed part: Thread features save
//! `print_clearance` (a length; not saved when none), and every wall of the groove is cut that
//! much further into the material (its flanks along their normals, its root deeper), a hole made
//! the tap drill that much wider each side. Older designs have none and are cut as they were. The
//! last release refuses a design with this change (the newer format number).
