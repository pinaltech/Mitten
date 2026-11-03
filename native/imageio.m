// Fast thumbnails through ImageIO: embedded previews or DCT-scaled decodes
// instead of decoding full-resolution originals. Pixels are BGRA, matching
// what GPUI uploads.
#import <Foundation/Foundation.h>
#import <ImageIO/ImageIO.h>
#import <CoreGraphics/CoreGraphics.h>
#import <AVFoundation/AVFoundation.h>

// The synchronous AVAsset accessors are deprecated in newer SDKs, but they
// are the only ones available back to macOS 12 and these run on workers.
#pragma clang diagnostic push
#pragma clang diagnostic ignored "-Wdeprecated-declarations"
static BOOL is_video(NSURL *url) {
    NSString *ext = url.pathExtension.lowercaseString;
    return [ext isEqualToString:@"mov"] || [ext isEqualToString:@"mp4"] || [ext isEqualToString:@"m4v"];
}
// A frame a little way in (first frames are often black), orientation applied.
static CGImageRef video_frame(NSURL *url, unsigned max_pixels) {
    AVURLAsset *asset = [AVURLAsset URLAssetWithURL:url options:nil];
    AVAssetImageGenerator *generator = [AVAssetImageGenerator assetImageGeneratorWithAsset:asset];
    generator.appliesPreferredTrackTransform = YES;
    generator.maximumSize = CGSizeMake(max_pixels, max_pixels);
    double seconds = CMTimeGetSeconds(asset.duration);
    CMTime at = CMTimeMakeWithSeconds(isfinite(seconds) ? MIN(1.0, seconds / 3) : 0, 600);
    return [generator copyCGImageAtTime:at actualTime:NULL error:nil];
}
int dp_video_info(const char *path, unsigned *width, unsigned *height, double *seconds) {
    @autoreleasepool {
        AVURLAsset *asset = [AVURLAsset URLAssetWithURL:[NSURL fileURLWithPath:[NSString stringWithUTF8String:path]] options:nil];
        AVAssetTrack *track = [asset tracksWithMediaType:AVMediaTypeVideo].firstObject;
        if (!track) return 0;
        CGSize size = CGSizeApplyAffineTransform(track.naturalSize, track.preferredTransform);
        *width = (unsigned)fabs(size.width);
        *height = (unsigned)fabs(size.height);
        double length = CMTimeGetSeconds(asset.duration);
        *seconds = isfinite(length) ? length : 0;
        return 1;
    }
}
#pragma clang diagnostic pop

// Draws `image` centre-cropped to fill an edge×edge BGRA buffer.
int dp_fill_square(CGImageRef image, unsigned edge, uint8_t *pixels) {
    if (!image || edge == 0) return 0;
    CGColorSpaceRef space = CGColorSpaceCreateDeviceRGB();
    CGContextRef context = CGBitmapContextCreate(pixels, edge, edge, 8, edge * 4, space,
        kCGImageAlphaPremultipliedFirst | kCGBitmapByteOrder32Little);
    CGColorSpaceRelease(space);
    if (!context) return 0;
    CGFloat w = CGImageGetWidth(image), h = CGImageGetHeight(image);
    CGFloat scale = MAX(edge / MAX(w, 1), edge / MAX(h, 1));
    CGRect rect = CGRectMake((edge - w * scale) / 2, (edge - h * scale) / 2, w * scale, h * scale);
    CGContextSetInterpolationQuality(context, kCGInterpolationMedium);
    CGContextDrawImage(context, rect, image);
    CGContextRelease(context);
    return 1;
}

static CGImageRef thumbnail(const char *path, unsigned max_pixels, bool embedded) {
    NSURL *url = [NSURL fileURLWithPath:[NSString stringWithUTF8String:path]];
    if (is_video(url)) return video_frame(url, max_pixels);
    CGImageSourceRef source = CGImageSourceCreateWithURL((__bridge CFURLRef)url,
        (__bridge CFDictionaryRef)@{(id)kCGImageSourceShouldCache: @NO});
    if (!source) return NULL;
    NSDictionary *options = @{
        (id)(embedded ? kCGImageSourceCreateThumbnailFromImageIfAbsent
                      : kCGImageSourceCreateThumbnailFromImageAlways): @YES,
        (id)kCGImageSourceCreateThumbnailWithTransform: @YES,
        (id)kCGImageSourceShouldCacheImmediately: @YES,
        (id)kCGImageSourceThumbnailMaxPixelSize: @(max_pixels),
    };
    CGImageRef image = CGImageSourceCreateThumbnailAtIndex(source, 0, (__bridge CFDictionaryRef)options);
    CFRelease(source);
    return image;
}

// A centre-cropped square for dense grids. Small squares may come straight from
// the file's embedded preview, which avoids decoding the image at all.
int dp_image_square(const char *path, unsigned edge, uint8_t *pixels) {
    @autoreleasepool {
        CGImageRef image = thumbnail(path, edge * 2, edge <= 160);
        int ok = dp_fill_square(image, edge, pixels);
        if (image) CGImageRelease(image);
        return ok;
    }
}

// An orientation-corrected JPEG at most `edge` px on its long side.
int dp_image_thumbnail(const char *path, const char *destination, unsigned edge) {
    @autoreleasepool {
        CGImageRef image = thumbnail(path, edge, false);
        if (!image) return 0;
        NSURL *url = [NSURL fileURLWithPath:[NSString stringWithUTF8String:destination]];
        CGImageDestinationRef out = CGImageDestinationCreateWithURL((__bridge CFURLRef)url,
            (__bridge CFStringRef)@"public.jpeg", 1, NULL);
        int ok = 0;
        if (out) {
            CGImageDestinationAddImage(out, image,
                (__bridge CFDictionaryRef)@{(id)kCGImageDestinationLossyCompressionQuality: @0.88});
            ok = CGImageDestinationFinalize(out) ? 1 : 0;
            CFRelease(out);
        }
        CGImageRelease(image);
        return ok;
    }
}
