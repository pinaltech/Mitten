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
// view's top left. `kind` 0 plays a file path, 1 a PhotoKit asset identifier.
// `visible == 0` hides and pauses it; an empty identifier also releases it.
void dp_video_sync(void *ns_view, int kind, const char *identifier, double x, double y, double w,
                   double h, int visible) {
    @autoreleasepool {
        NSView *host = (__bridge NSView *)ns_view;
        NSString *key = [NSString stringWithFormat:@"%d:%s", kind, identifier ?: ""];
        if (!identifier || !*identifier) {
            if (player_view) {
                stop();
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
        if (hidden && !player_view.hidden) [player_view.player pause];
        player_view.hidden = hidden;
    }
}
