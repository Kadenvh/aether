---------------------------- MODULE ledger ----------------------------
(* AETHER USL state machine (U14, R14): the append-only, hash-chained event
   ledger. This offline model proves the ledger's *structural* invariants hold
   under all interleavings — distinct from the runtime Z3 gate (U7), which
   proves individual mutation values. We model the chain abstractly: each event
   carries an index and a link to its predecessor's "hash"; we check that the
   log only ever grows, indices are dense and monotonic, and every link points
   to the immediately preceding event (chain integrity). *)

EXTENDS Naturals, Sequences

CONSTANT
    \* @type: Int;
    MaxLen               \* bound the model so Apalache/TLC terminate

VARIABLE
    \* @type: Seq({ idx: Int, prev: Int });
    log                  \* sequence of records: [idx |-> Nat, prev |-> Nat]

(* The genesis link value (no predecessor). *)
Genesis == 0

Init == log = << >>

(* Append one event: its idx is the next dense position (1-based), and its
   prev-link is the previous event's idx (or Genesis for the first event). *)
AppendEvent ==
    /\ Len(log) < MaxLen
    /\ LET n == Len(log) + 1
           prevLink == IF n = 1 THEN Genesis ELSE log[n-1].idx
       IN log' = Append(log, [idx |-> n, prev |-> prevLink])

(* Append while there's room; once full the ledger idles (stutter) rather than
   deadlocking — the invariant is still checked in the terminal state. *)
Next == AppendEvent \/ (Len(log) = MaxLen /\ UNCHANGED log)

(* Stutter when full so behaviours are infinite (no deadlock flagged).
   `vars == log` (not `<<log>>`) avoids 1-tuple/Seq ambiguity with one var. *)
vars == log
Spec == Init /\ [][Next \/ UNCHANGED vars]_vars

-----------------------------------------------------------------------
(* Invariants *)

(* Indices are dense and monotonic: event at position i has idx = i. *)
DenseMonotonic ==
    \A i \in 1..Len(log) : log[i].idx = i

(* Chain integrity: the first event links to Genesis; every later event links
   to its immediate predecessor's idx. No event can be reordered or dropped
   without breaking a link. *)
ChainIntact ==
    \A i \in 1..Len(log) :
        log[i].prev = (IF i = 1 THEN Genesis ELSE log[i-1].idx)

(* Append-only: the log never shrinks (checked as an action property). *)
NeverShrinks == Len(log') >= Len(log)

(* The conjunction we ask Apalache/TLC to verify as a state invariant. *)
Inv == DenseMonotonic /\ ChainIntact
=======================================================================
