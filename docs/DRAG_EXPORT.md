# Original-file drag export

On macOS, GPUI handles selection and album drags inside the app. Leaving the
source window, including crossing into an overlapping window, hands the drag
to AppKit. A first coalesced movement that already leaves the window also works.
The fitted viewer offers the same drag; above 100%, dragging pans the image.

Each photo supplies an `NSFilePromiseProvider` with a valid image UTI and a
sanitized, unique basename. Existing local files also advertise a standard file
URL. No original is requested while constructing or cancelling a drag. The
provider's background queue asks the captured library driver to export the
original only when the receiver requests it. The driver remains available if
the user switches libraries. Promise writers retain their delegate and Rust
context until AppKit releases them. The native drag source stays alive until
the session ends, then clears GPUI's pending mouse press.

AppKit already coordinates the destination write around the promise delegate.
The delegate must write directly and call completion exactly once. An earlier
version nested a second `NSFileCoordinator` inside that callback: a process
sample showed it waiting for AppKit's existing write claim. Finder displayed the
green plus but received no file. Removing that nested claim fixes the stall.
Apple's [file-promise sample](https://developer.apple.com/documentation/appkit/supporting-table-view-drag-and-drop-through-file-promises)
also performs the file copy directly in its delegate.

Portable fulfillment stages the driver's export, checks for one regular file,
preserves its bytes, and atomically persists it using the receiver's chosen
filename without overwriting. Temporary files are cleaned on failure. Connected
sources use the same export path as the Export command.

Regression coverage includes original bytes, duplicate names, export failure,
cleanup, overwrite refusal, deferred connected-source fetches, FFI errors and
context release. The native test validates a private pasteboard promise and
runs its delegate inside a coordinated write to guard against the deadlock; it
does not synthesize a receiver's drag callback. It needs macOS pasteboard and
LaunchServices access. The user separately confirmed a real Finder drop creates
the file in the rebuilt app. Linux/Windows external drag UI remains unverified;
portable fulfillment and provider adapters compile on both.
