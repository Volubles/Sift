# Session navigation

Sift remembers recently used views while the window is open. Returning to a view
restores its position and prepares its selected sound without starting playback.
Clicking a sound or navigating with W/S or Up/Down still auditions it when
auto-play is enabled.

Auditioning a visible row leaves the viewport in place. Keyboard movement past
an edge reveals the next row with the smallest necessary scroll, preserving your
position rather than centering each newly activated sound.

## Remembered state

| View | State |
| --- | --- |
| Library | Last folder, expanded branches, folder paging, and tree scroll |
| Library or folder sound table | Search text, completed marks, active sound, and scroll position |
| Collection | Completed marks, active sound, content page, and scroll position |
| Favorites | Its independent search, completed marks, active sound, and scroll position |

Folders are identified by their native paths within a library, and collections
by their durable IDs. Similarly named folders or libraries keep separate state.
A previously unvisited folder inherits the search within its current library;
returning to a visited folder restores that folder's own search. A new library
starts with an empty search.

Changing search text resets that view's marks and scroll. Completed marks survive
navigation and refresh. Unfinished range selection and queued keyboard movement
are cancelled when their view is left; late replies cannot update another view.
The initial loaded library list receives focus once when no other control has
it, so keyboard audition works without a first click. After that, keyboard focus
remains with the controls you use rather than jumping into a list.

The Collections add button continues to use marks from the last used sound list.
Choosing an add destination retains the previous source marks even when that
destination restores its own view. Making new marks there changes the source for
subsequent group actions. Accepted saves retain their original destination.

## Loading and limits

Returning to a library/folder or collection requests a fresh bounded result page.
Restored sources are revalidated by the existing workers. A missing source stays
distinct from an empty search; Sift does not substitute another sound. Confirmed
collection removals update cached marks, and deleted collections and detached
library scopes discard their cached state.

Inactive library/folder views, library trees, and collection views each have a
16-view cache with an 8 MiB estimated retained-state budget. The estimate includes
selection path data and set capacity; location keys and fixed container overhead
are additional. Result pages, audio, waveform data, workers, and pending saves are
not stored in these caches. Oversized states are not retained; the least recently
used views are evicted as needed. An evicted view opens with its initial state.

This memory is local to the open window. Restoring a session after closing Sift
and back/forward navigation history are separate future features. Volume and
auto-play remain window-wide settings.
