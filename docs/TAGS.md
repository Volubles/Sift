# Manual tags

The tag store holds global labels and their native source associations in
`catalog.tags.sqlite`. Tag identities are independent of rebuildable catalog
rows, collection membership and favorites. Creating or removing an association
does not inspect, rename or write the source audio. Missing sources retain tags;
a replacement locator is not inferred automatically.

Names contain 1–40 characters, are trimmed and compared using a lowercase key.
Reusing a name adds its existing tag. Each sound supports 32 tags; edits accept
up to 128 sounds and a 1 MiB combined native-locator budget. Batch additions and
removals are transactions. A failed addition rolls back every association and
any newly created definition, preserving unrelated tags. Empty definitions remain
available for reuse after removing their last association.

The version-1 database rejects unsupported versions and foreign populated files
without resetting their contents. One background owner has four-slot command and
event queues. Reads have independent generations; accepted edits drain at shutdown.
Visible membership reads are bounded to 641 sources and 32 tags per source;
suggestions return at most 24 names. Selection summaries resolve the same captured
library, favorite or collection intent as other group actions.

Library queries carry up to eight unique tag identities. All selected tags must
match, together with text, scope and metadata filters. Pages, marks, ranges,
identity lookup and outgoing copies share those predicates. A lazy read-only
attachment uses the tag/source index to join the first tag, with indexed membership
checks for additional tags. Count and page snapshots observe both databases;
commits to either invalidate retained counts and anchors. No audio data or full
result-ID list is loaded to filter tags. Ordinary unfiltered search does not
require attaching tag storage.

The durable store and query integration are implemented. Compact desktop chips,
the tag editor and interactive filtering are the next part of this feature.
