// The viewer's video player: one AVPlayerView laid over the GPUI view at the
// fitted photo frame. GPUI cannot decode video, so playback stays native.
// Every entry point runs on the main thread (it is called while rendering).
#import <AppKit/AppKit.h>
#import <AVKit/AVKit.h>
#import <AVFoundation/AVFoundation.h>
#import <Photos/Photos.h>

static AVPlayerView *player_view;
static NSString *current;
static PHImageRequestID pending = PHInvalidImageRequestID;

// A hidden player must not keep keyboard focus: key events would go to it
// (beeping) instead of GPUI's view.
static void release_focus(void) {
    NSWindow *window = player_view.window;
    NSResponder *responder = window.firstResponder;
    if ([responder isKindOfClass:[NSView class]] && [(NSView *)responder isDescendantOf:player_view])
        [window makeFirstResponder:player_view.superview];
}

static void stop(void) {
    if (pending != PHInvalidImageRequestID) [[PHImageManager defaultManager] cancelImageRequest:pending];
    pending = PHInvalidImageRequestID;
    [player_view.player pause];
    player_view.player = nil;
    current = nil;
}

static void play(AVPlayerItem *item) {
    player_view.player = [AVPlayer playerWithPlayerItem:item];
    [player_view.player play];
}

// Shows (or moves) the player over `ns_view` at x/y/w/h, points from the
// view's top left. `kind` 0 plays a file path, 1 a PhotoKit asset identifier,
// 2 a server URL streamed with `headers` ("Name: value" lines).
// `visible == 0` hides and pauses it; an empty identifier also releases it.
void dp_video_sync(void *ns_view, int kind, const char *identifier, const char *headers, double x,
                   double y, double w, double h, int visible) {
    @autoreleasepool {
        NSView *host = (__bridge NSView *)ns_view;
        NSString *key = [NSString stringWithFormat:@"%d:%s", kind, identifier ?: ""];
        if (!identifier || !*identifier) {
            if (player_view) {
                stop();
                release_focus();
                player_view.hidden = YES;
            }
            return;
        }
        if (!player_view) {
            player_view = [[AVPlayerView alloc] initWithFrame:NSZeroRect];
            player_view.controlsStyle = AVPlayerViewControlsStyleFloating;
            player_view.videoGravity = AVLayerVideoGravityResizeAspect;
            player_view.wantsLayer = YES;
            player_view.layer.backgroundColor = NSColor.clearColor.CGColor;
        }
        if (player_view.superview != host) {
            [player_view removeFromSuperview];
            [host addSubview:player_view positioned:NSWindowAbove relativeTo:nil];
        }
        CGFloat top = host.isFlipped ? y : host.bounds.size.height - y - h;
        NSRect frame = NSMakeRect(x, top, w, h);
        if (!NSEqualRects(player_view.frame, frame)) player_view.frame = frame;
        if (![current isEqualToString:key]) {
            stop();
            current = key;
            NSString *value = [NSString stringWithUTF8String:identifier];
            if (kind == 0) {
                play([AVPlayerItem playerItemWithURL:[NSURL fileURLWithPath:value]]);
            } else if (kind == 2) {
                NSURL *url = [NSURL URLWithString:value];
                NSMutableDictionary *fields = [NSMutableDictionary new];
                for (NSString *line in [[NSString stringWithUTF8String:headers ?: ""] componentsSeparatedByString:@"\n"]) {
                    NSRange colon = [line rangeOfString:@": "];
                    if (colon.location != NSNotFound)
                        fields[[line substringToIndex:colon.location]] = [line substringFromIndex:NSMaxRange(colon)];
                }
                if (url) {
                    // AVURLAsset sends these headers with every request of the stream.
                    AVURLAsset *asset = [AVURLAsset URLAssetWithURL:url options:@{@"AVURLAssetHTTPHeaderFieldsKey": fields}];
                    play([AVPlayerItem playerItemWithAsset:asset]);
                }
            } else {
                PHAsset *asset = [PHAsset fetchAssetsWithLocalIdentifiers:@[value] options:nil].firstObject;
                if (asset) {
                    PHVideoRequestOptions *options = [PHVideoRequestOptions new];
                    options.networkAccessAllowed = YES;
                    options.deliveryMode = PHVideoRequestOptionsDeliveryModeAutomatic;
                    pending = [[PHImageManager defaultManager] requestPlayerItemForVideo:asset options:options
                        resultHandler:^(AVPlayerItem *item, NSDictionary *info) {
                            (void)info;
                            dispatch_async(dispatch_get_main_queue(), ^{
                                if (item && [current isEqualToString:key]) {
                                    pending = PHInvalidImageRequestID;
                                    play(item);
                                }
                            });
                        }];
                }
            }
        }
        BOOL hidden = visible == 0;
        if (hidden && !player_view.hidden) {
            [player_view.player pause];
            release_focus();
        }
        player_view.hidden = hidden;
    }
}

// GPUI resigns key status when it sees a "spurious" windowDidBecomeKey (see
// gpui's window_did_change_key_status). When that misfires the window stays
// visibly frontmost but never key: hover and cursor changes stop (GPUI only
// tracks the pointer for the key window) and typing beeps, until the app is
// deactivated. Returns 1 after restoring key status to the window when the
// app is active and no other visible window, sheet or modal holds it.
int dp_window_restore_key(void *ns_view) {
    @autoreleasepool {
        NSView *view = (__bridge NSView *)ns_view;
        NSWindow *window = view.window;
        if (!window || window.isKeyWindow || !window.isVisible || window.isMiniaturized) return 0;
        if (!NSApp.isActive || NSApp.modalWindow || window.attachedSheet) return 0;
        NSWindow *key = NSApp.keyWindow;
        if (key && key != window && key.isVisible) return 0;
        // Only when the pointer is over this window, as the user returns to it.
        if (!NSPointInRect(NSEvent.mouseLocation, window.frame)) return 0;
        [window makeKeyWindow];
        if (!window.isKeyWindow) {
            // AppKit still believes it is key: balance GPUI's resign.
            [window becomeKeyWindow];
        }
        if (window.firstResponder != view && ![window.firstResponder isKindOfClass:[NSView class]])
            [window makeFirstResponder:view];
        return window.isKeyWindow ? 1 : 0;
    }
}
