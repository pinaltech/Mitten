#import <AppKit/AppKit.h>
#import <objc/runtime.h>
#include <stdlib.h>
#include <stdio.h>
static void DPTrace(const char *message) { if (getenv("DESERT_PHOTOS_TRACE_DRAGS")) fprintf(stderr,"Photos drag: %s\n",message); }
#import <UniformTypeIdentifiers/UniformTypeIdentifiers.h>
#import <ImageIO/ImageIO.h>

typedef bool (*DPWriteOriginal)(void *, size_t, const char *, char *, size_t);
typedef void (*DPReleaseContext)(void *);

@interface DPDragContext : NSObject
@property(nonatomic) void *rust;
@property(nonatomic) DPWriteOriginal write;
@property(nonatomic) DPReleaseContext releaseContext;
// Originals fetched from a remote source to serve image data or a file URL.
@property(nonatomic, copy) NSString *temporaryDirectory;
@end
@implementation DPDragContext
- (void)dealloc {
    if (_rust && _releaseContext) _releaseContext(_rust);
    if (_temporaryDirectory) [[NSFileManager defaultManager] removeItemAtPath:_temporaryDirectory error:NULL];
}
@end

// Re-encode an image file as `type`. Orientation is applied to the pixels, so
// receivers that ignore EXIF orientation still show the photo upright, and
// transparency is flattened onto white for JPEG. Metadata stays with the original.
static NSData *DPConvertImage(NSString *path, NSString *type) {
    CGImageSourceRef source = CGImageSourceCreateWithURL((__bridge CFURLRef)[NSURL fileURLWithPath:path], NULL);
    if (!source) return nil;
    NSDictionary *properties = (__bridge_transfer NSDictionary *)CGImageSourceCopyPropertiesAtIndex(source, 0, NULL);
    NSInteger width = [properties[(id)kCGImagePropertyPixelWidth] integerValue];
    NSInteger height = [properties[(id)kCGImagePropertyPixelHeight] integerValue];
    CGImageRef image = NULL;
    if (width > 0 && height > 0) {
        NSDictionary *options = @{
            (id)kCGImageSourceCreateThumbnailFromImageAlways: @YES,
            (id)kCGImageSourceCreateThumbnailWithTransform: @YES,
            (id)kCGImageSourceThumbnailMaxPixelSize: @(MAX(width, height)),
            (id)kCGImageSourceShouldCacheImmediately: @YES,
        };
        image = CGImageSourceCreateThumbnailAtIndex(source, 0, (__bridge CFDictionaryRef)options);
    }
    if (!image) image = CGImageSourceCreateImageAtIndex(source, 0, NULL);
    CFRelease(source);
    if (!image) return nil;
    BOOL jpeg = [type isEqualToString:UTTypeJPEG.identifier];
    CGImageAlphaInfo alpha = CGImageGetAlphaInfo(image);
    if (jpeg && alpha != kCGImageAlphaNone && alpha != kCGImageAlphaNoneSkipFirst && alpha != kCGImageAlphaNoneSkipLast) {
        size_t w = CGImageGetWidth(image), h = CGImageGetHeight(image);
        CGColorSpaceRef space = CGImageGetColorSpace(image);
        if (!space || CGColorSpaceGetModel(space) != kCGColorSpaceModelRGB) space = NULL;
        CGColorSpaceRef rgb = space ? CGColorSpaceRetain(space) : CGColorSpaceCreateWithName(kCGColorSpaceSRGB);
        CGContextRef bitmap = CGBitmapContextCreate(NULL, w, h, 8, 0, rgb, (CGBitmapInfo)kCGImageAlphaNoneSkipLast);
        CGColorSpaceRelease(rgb);
        if (bitmap) {
            CGContextSetRGBFillColor(bitmap, 1, 1, 1, 1);
            CGContextFillRect(bitmap, CGRectMake(0, 0, w, h));
            CGContextDrawImage(bitmap, CGRectMake(0, 0, w, h), image);
            CGImageRef flat = CGBitmapContextCreateImage(bitmap);
            CGContextRelease(bitmap);
            if (flat) { CGImageRelease(image); image = flat; }
        }
    }
    NSMutableData *data = [NSMutableData new];
    CGImageDestinationRef destination =
        CGImageDestinationCreateWithData((__bridge CFMutableDataRef)data, (__bridge CFStringRef)type, 1, NULL);
    if (!destination) { CGImageRelease(image); return nil; }
    NSDictionary *options = jpeg ? @{(id)kCGImageDestinationLossyCompressionQuality: @0.92} : @{};
    CGImageDestinationAddImage(destination, image, (__bridge CFDictionaryRef)options);
    BOOL ok = CGImageDestinationFinalize(destination);
    CFRelease(destination);
    CGImageRelease(image);
    return ok ? data : nil;
}

// The image flavors offered for one photo: its original type, then the
// formats nearly every receiver reads (JPEG for web and chat, PNG for
// browsers and Electron, TIFF for classic Cocoa). Data is produced only when
// a receiver asks for that type, then cached for the life of the item.
@interface DPFlavors : NSObject
@property(nonatomic, strong) DPDragContext *context;
@property(nonatomic) NSUInteger index;
@property(nonatomic, copy) NSString *filename;
@property(nonatomic, copy) NSString *localPath;
@property(nonatomic, strong) NSMutableDictionary<NSString *, NSData *> *cache;
@property(nonatomic, copy) NSString *fetchedPath;
@end
@implementation DPFlavors
- (NSString *)originalType {
    UTType *type = [UTType typeWithFilenameExtension:self.filename.pathExtension];
    return [type conformsToType:UTTypeImage] ? type.identifier : nil;
}
- (NSArray<NSPasteboardType> *)imageTypes {
    NSMutableOrderedSet *types = [NSMutableOrderedSet new];
    NSString *original = self.originalType;
    if (original) [types addObject:original];
    [types addObjectsFromArray:@[UTTypeJPEG.identifier, UTTypePNG.identifier, UTTypeTIFF.identifier]];
    return types.array;
}
// A local path to the original, fetching it from the source when needed.
- (NSString *)originalPath {
    if (self.localPath && [[NSFileManager defaultManager] fileExistsAtPath:self.localPath]) return self.localPath;
    @synchronized(self) {
        if (self.fetchedPath) return self.fetchedPath;
        NSString *root;
        @synchronized(self.context) {
            if (!self.context.temporaryDirectory)
                self.context.temporaryDirectory = [NSTemporaryDirectory() stringByAppendingPathComponent:
                    [@"DesertPhotos-" stringByAppendingString:NSUUID.UUID.UUIDString]];
            root = self.context.temporaryDirectory;
        }
        NSString *folder = [root stringByAppendingPathComponent:[NSString stringWithFormat:@"%lu", (unsigned long)self.index]];
        if (![[NSFileManager defaultManager] createDirectoryAtPath:folder withIntermediateDirectories:YES attributes:nil error:NULL]) return nil;
        NSString *path = [folder stringByAppendingPathComponent:self.filename];
        char error[4096] = {0};
        if (!self.context.write(self.context.rust, self.index, path.fileSystemRepresentation, error, sizeof(error))) {
            DPTrace(error);
            return nil;
        }
        self.fetchedPath = path;
        return path;
    }
}
- (NSData *)dataForType:(NSPasteboardType)type {
    @synchronized(self) {
        if (!self.cache) self.cache = [NSMutableDictionary new];
        NSData *data = self.cache[type];
        if (data) return data;
        NSString *path = self.originalPath;
        if (!path) return nil;
        data = [type isEqualToString:self.originalType] ? [NSData dataWithContentsOfFile:path] : DPConvertImage(path, type);
        if (data) self.cache[type] = data;
        return data;
    }
}
@end

// A copied photo: the original as a file URL, plus each image flavor.
@interface DPPhotoItem : NSObject <NSPasteboardWriting>
@property(nonatomic, strong) DPFlavors *flavors;
@end
@implementation DPPhotoItem
- (NSArray<NSPasteboardType> *)writableTypesForPasteboard:(NSPasteboard *)pasteboard {
    (void)pasteboard;
    return [@[NSPasteboardTypeFileURL] arrayByAddingObjectsFromArray:self.flavors.imageTypes];
}
- (NSPasteboardWritingOptions)writingOptionsForType:(NSPasteboardType)type pasteboard:(NSPasteboard *)pasteboard {
    (void)type; (void)pasteboard;
    return NSPasteboardWritingPromised;
}
- (id)pasteboardPropertyListForType:(NSPasteboardType)type {
    if ([type isEqualToString:NSPasteboardTypeFileURL]) {
        NSString *path = self.flavors.originalPath;
        return path ? [NSURL fileURLWithPath:path].absoluteString : nil;
    }
    return [self.flavors dataForType:type];
}
@end

@interface DPFileWriter : NSObject <NSFilePromiseProviderDelegate>
@property(nonatomic, strong) DPDragContext *context;
@property(nonatomic, copy) NSString *filename;
@property(nonatomic) NSUInteger index;
@end
@implementation DPFileWriter
- (NSString *)filePromiseProvider:(NSFilePromiseProvider *)provider fileNameForType:(NSString *)type {
    (void)provider; (void)type;
    return self.filename;
}
- (NSOperationQueue *)operationQueueForFilePromiseProvider:(NSFilePromiseProvider *)provider {
    (void)provider;
    static NSOperationQueue *queue;
    static dispatch_once_t once;
    dispatch_once(&once, ^{
        queue = [NSOperationQueue new];
        queue.name = @"Photos original-file promises";
        queue.maxConcurrentOperationCount = 2;
    });
    return queue;
}
- (void)filePromiseProvider:(NSFilePromiseProvider *)provider
         writePromiseToURL:(NSURL *)url
         completionHandler:(void (^)(NSError *))completionHandler {
    (void)provider;
    DPTrace("original requested by receiver");
    // NSFilePromiseProvider already holds the coordinated write claim while
    // invoking this delegate. A second coordinator here deadlocks against it.
    NSError *writeError = nil;
    char error[4096] = {0};
    if (!self.context.write(self.context.rust, self.index, url.fileSystemRepresentation, error, sizeof(error))) {
        NSString *message = [NSString stringWithUTF8String:error] ?: @"Cannot export the original photo.";
        writeError = [NSError errorWithDomain:@"com.pinal.DesertPhotos.Drag" code:1
            userInfo:@{NSLocalizedDescriptionKey: message}];
    }
    DPTrace(writeError ? "original fulfillment failed" : "original delivered");
    completionHandler(writeError);
}
@end

// A strong delegate is essential: promises are fulfilled AFTER the drag ends.
// Local items advertise a standard file URL as well as the file promise.
// All advertised types must be UTIs; legacy NSFilenamesPboardType is invalid here.
// Image flavors let receivers that take pictures but not files (or not HEIC) accept the drop.
@interface DPFilePromise : NSFilePromiseProvider
@property(nonatomic, strong) DPFileWriter *writer;
@property(nonatomic, copy) NSString *localPath;
@property(nonatomic, strong) DPFlavors *flavors;
@end
@implementation DPFilePromise
- (NSArray<NSPasteboardType> *)writableTypesForPasteboard:(NSPasteboard *)pasteboard {
    NSMutableArray *types = [[super writableTypesForPasteboard:pasteboard] mutableCopy];
    if (self.localPath) [types addObjectsFromArray:@[NSPasteboardTypeFileURL]];
    for (NSPasteboardType type in self.flavors.imageTypes)
        if (![types containsObject:type]) [types addObject:type];
    return types;
}
- (NSPasteboardWritingOptions)writingOptionsForType:(NSPasteboardType)type pasteboard:(NSPasteboard *)pasteboard {
    if ([self.flavors.imageTypes containsObject:type]) return NSPasteboardWritingPromised;
    return [super writingOptionsForType:type pasteboard:pasteboard];
}
- (id)pasteboardPropertyListForType:(NSPasteboardType)type {
    if (self.localPath && [type isEqualToString:NSPasteboardTypeFileURL])
        return [NSURL fileURLWithPath:self.localPath].absoluteString;
    if ([self.flavors.imageTypes containsObject:type]) return [self.flavors dataForType:type];
    return [super pasteboardPropertyListForType:type];
}
@end

static DPFlavors *DPMakeFlavors(DPDragContext *context, NSDictionary *record, NSUInteger index) {
    DPFlavors *flavors = [DPFlavors new];
    flavors.context = context;
    flavors.index = index;
    flavors.filename = record[@"name"];
    flavors.localPath = record[@"local"];
    return flavors;
}
static NSArray *DPRecords(const char *json) {
    NSData *data = [[NSString stringWithUTF8String:json] dataUsingEncoding:NSUTF8StringEncoding];
    NSArray *records = data ? [NSJSONSerialization JSONObjectWithData:data options:0 error:NULL] : nil;
    return [records isKindOfClass:[NSArray class]] && records.count > 0 ? records : nil;
}
static NSArray<DPPhotoItem *> *DPPhotoItems(DPDragContext *context, NSArray *records) {
    NSMutableArray *items = [NSMutableArray new];
    for (NSUInteger index = 0; index < records.count; index++) {
        DPPhotoItem *item = [DPPhotoItem new];
        item.flavors = DPMakeFlavors(context, records[index], index);
        [items addObject:item];
    }
    return items;
}

// Put photos on the general pasteboard. Always consumes rustContext; the
// pasteboard owns the items (and so the context) until its contents change.
bool dp_copy_photos(const char *json, void *rustContext, DPWriteOriginal write, DPReleaseContext releaseContext) {
    DPDragContext *context = [DPDragContext new];
    context.rust = rustContext; context.write = write; context.releaseContext = releaseContext;
    @try {
        NSArray *records = DPRecords(json);
        if (!records) return false;
        NSPasteboard *pasteboard = [NSPasteboard generalPasteboard];
        [pasteboard clearContents];
        return [pasteboard writeObjects:DPPhotoItems(context, records)];
    } @catch (NSException *exception) {
        if (getenv("DESERT_PHOTOS_TRACE_DRAGS")) NSLog(@"Photos copy exception: %@", exception.reason);
        return false;
    }
}

// Write copied items to a private pasteboard and read every flavor back as a
// receiver would. Each image flavor must decode to `width` x `height`.
bool dp_verify_photo_flavors(const char *json, void *rustContext, DPWriteOriginal write,
                             DPReleaseContext releaseContext, size_t width, size_t height) {
    @autoreleasepool {
        DPDragContext *context = [DPDragContext new];
        context.rust = rustContext; context.write = write; context.releaseContext = releaseContext;
        NSArray *records = DPRecords(json);
        if (!records) return false;
        NSPasteboard *pasteboard = [NSPasteboard pasteboardWithUniqueName];
        BOOL ok = [pasteboard writeObjects:DPPhotoItems(context, records)];
        for (NSPasteboardItem *item in pasteboard.pasteboardItems) {
            if (!ok) break;
            NSString *url = [item stringForType:NSPasteboardTypeFileURL];
            ok = url && [[NSFileManager defaultManager] fileExistsAtPath:[NSURL URLWithString:url].path];
            for (NSString *type in @[UTTypeJPEG.identifier, UTTypePNG.identifier, UTTypeTIFF.identifier]) {
                NSData *data = ok ? [item dataForType:type] : nil;
                CGImageSourceRef source = data ? CGImageSourceCreateWithData((__bridge CFDataRef)data, NULL) : NULL;
                NSString *decoded = source ? (__bridge NSString *)CGImageSourceGetType(source) : nil;
                CGImageRef image = source ? CGImageSourceCreateImageAtIndex(source, 0, NULL) : NULL;
                ok = image && [decoded isEqualToString:type]
                    && CGImageGetWidth(image) == width && CGImageGetHeight(image) == height;
                if (image) CGImageRelease(image);
                if (source) CFRelease(source);
            }
        }
        ok = ok && pasteboard.pasteboardItems.count == records.count;
        [pasteboard releaseGlobally];
        return ok;
    }
}

static char DPDragSourceKey;
@interface DPDragSource : NSObject <NSDraggingSource>
@property(nonatomic, weak) NSView *view;
@property(nonatomic, strong) NSDraggingSession *session;
@end
@implementation DPDragSource
- (NSDragOperation)draggingSession:(NSDraggingSession *)session
    sourceOperationMaskForDraggingContext:(NSDraggingContext)context {
    (void)session; (void)context;
    return NSDragOperationCopy;
}
- (BOOL)ignoreModifierKeysForDraggingSession:(NSDraggingSession *)session { (void)session; return YES; }
- (void)draggingSession:(NSDraggingSession *)session endedAtPoint:(NSPoint)point operation:(NSDragOperation)operation {
    (void)session; (void)point;
    __attribute__((objc_precise_lifetime)) DPDragSource *source = self;
    DPTrace(operation == NSDragOperationCopy ? "session copied" : "session cancelled");
    source.session = nil;
    NSView *view = source.view;
    if (view.window) {
        // AppKit consumes the physical mouse-up during its drag session. Tell
        // GPUI's source view to clear pending presses outside every hitbox so a
        // later hover cannot start a phantom drag. This is an internal view
        // notification, not an event posted to the system or the drop target.
        NSEvent *up = [NSEvent mouseEventWithType:NSEventTypeLeftMouseUp
            location:NSMakePoint(-1., -1.) modifierFlags:0 timestamp:NSProcessInfo.processInfo.systemUptime
            windowNumber:view.window.windowNumber context:nil eventNumber:0 clickCount:1 pressure:0.];
        [view mouseUp:up];
        objc_setAssociatedObject(view, &DPDragSourceKey, nil, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
    }
}
@end

// A receiving window can overlap the source window's bounds. Don't require
// leaving the rectangle before handing the drag to AppKit.
bool dp_drag_is_over_other_window(void *nsView) {
    NSView *view = (__bridge NSView *)nsView;
    if (!view.window) return false;
    NSInteger target = [NSWindow windowNumberAtPoint:NSEvent.mouseLocation belowWindowWithWindowNumber:0];
    return target != 0 && target != view.window.windowNumber;
}

// Always consumes rustContext, including failure. No bytes are fetched here.
bool dp_start_original_drag(void *nsView, const char *json, void *rustContext,
                            DPWriteOriginal write, DPReleaseContext releaseContext) {
    DPDragContext *context = [DPDragContext new];
    context.rust = rustContext;
    context.write = write;
    context.releaseContext = releaseContext;
    @try {
        DPTrace("handoff requested");
        NSView *view = (__bridge NSView *)nsView;
        NSEvent *event = NSApp.currentEvent;
        if (!view || !event || event.type != NSEventTypeLeftMouseDragged) {DPTrace("no current native mouse-drag event");return false;}
        NSArray *records = DPRecords(json);
        if (!records) return false;
        NSMutableArray<NSDraggingItem *> *items = [NSMutableArray new];
        NSPoint location = [view convertPoint:event.locationInWindow fromView:nil];
        for (NSUInteger index = 0; index < records.count; index++) {
            NSDictionary *record = records[index];
            NSString *name = record[@"name"];
            DPFileWriter *writer = [DPFileWriter new];
            writer.context = context;
            writer.filename = name;
            writer.index = index;
            NSString *type = [UTType typeWithFilenameExtension:name.pathExtension].identifier ?: UTTypeImage.identifier;
            DPFilePromise *promise = [[DPFilePromise alloc] initWithFileType:type delegate:writer];
            promise.writer = writer;
            promise.localPath = record[@"local"];
            promise.flavors = DPMakeFlavors(context, record, index);
            NSDraggingItem *item = [[NSDraggingItem alloc] initWithPasteboardWriter:promise];
            NSString *preview = record[@"preview"];
            NSImage *image = preview ? [[NSImage alloc] initWithContentsOfFile:preview] : nil;
            if (!image) image = [[NSWorkspace sharedWorkspace] iconForContentType:UTTypeImage];
            [item setDraggingFrame:NSMakeRect(location.x - 32., location.y - 32., 64., 64.) contents:image];
            [items addObject:item];
        }
        DPDragSource *source = [DPDragSource new];
        // AppKit does not retain draggingSource. The source view owns it until
        // endedAtPoint, and the source keeps the session alive for that duration.
        source.view = view;
        objc_setAssociatedObject(view, &DPDragSourceKey, source, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
        source.session = [view beginDraggingSessionWithItems:items event:event source:source];
        if (!source.session) {
            objc_setAssociatedObject(view, &DPDragSourceKey, nil, OBJC_ASSOCIATION_RETAIN_NONATOMIC);
            return false;
        }
        source.session.draggingFormation = NSDraggingFormationPile;
        DPTrace("native session started");
        return true;
    } @catch (NSException *exception) {
        if (getenv("DESERT_PHOTOS_TRACE_DRAGS")) NSLog(@"Photos drag exception: %@",exception.reason);
        return false;
    }
}

// Validate the AppKit pasteboard producer and fulfill its delegate on the
// configured background queue. Receiver APIs require an actual drag-operation
// callback, so this deliberately tests fulfillment without simulating a drop.
// The private pasteboard never touches the user's clipboard.
bool dp_verify_original_promise(const char *directory, void *rustContext,
                               DPWriteOriginal write, DPReleaseContext releaseContext) {
    @autoreleasepool {
      @try {
        DPDragContext *context = [DPDragContext new];
        context.rust = rustContext; context.write = write; context.releaseContext = releaseContext;
        DPFileWriter *writer = [DPFileWriter new];
        writer.context = context; writer.filename = @"Promised.png"; writer.index = 0;
        DPFilePromise *promise = [[DPFilePromise alloc] initWithFileType:UTTypePNG.identifier delegate:writer];
        promise.writer = writer;
        NSPasteboard *pasteboard = [NSPasteboard pasteboardWithUniqueName];
        if (![pasteboard writeObjects:@[promise]]) { [pasteboard releaseGlobally]; return false; }
        NSArray<NSFilePromiseReceiver *> *receivers = [pasteboard readObjectsForClasses:@[[NSFilePromiseReceiver class]] options:@{}];
        if (receivers.count != 1) { [pasteboard releaseGlobally]; return false; }
        NSURL *folder = [NSURL fileURLWithPath:[NSString stringWithUTF8String:directory] isDirectory:YES];
        NSURL *destination = [folder URLByAppendingPathComponent:writer.filename];
        NSOperationQueue *queue = [writer operationQueueForFilePromiseProvider:promise];
        NSCondition *condition = [NSCondition new];
        __block BOOL complete = NO, succeeded = NO;
        [queue addOperationWithBlock:^{
            // Match AppKit's coordinated write around its delegate invocation.
            // Nested coordination inside DPFileWriter must never be reintroduced.
            NSFileCoordinator *coordinator = [[NSFileCoordinator alloc] initWithFilePresenter:nil];
            NSError *coordinationError = nil;
            [coordinator coordinateWritingItemAtURL:destination options:0 error:&coordinationError byAccessor:^(NSURL *file) {
                [writer filePromiseProvider:promise writePromiseToURL:file completionHandler:^(NSError *error) {
                    [condition lock];
                    succeeded = !error && [[NSFileManager defaultManager] fileExistsAtPath:file.path];
                    complete = YES; [condition signal]; [condition unlock];
                }];
            }];
            if (coordinationError) {
                [condition lock]; complete = YES; [condition signal]; [condition unlock];
            }
        }];
        NSDate *deadline = [NSDate dateWithTimeIntervalSinceNow:10.];
        [condition lock];
        while (!complete && [condition waitUntilDate:deadline]) {}
        BOOL result = complete && succeeded;
        [condition unlock];
        [pasteboard releaseGlobally];
        return result;
      } @catch (NSException *exception) {NSLog(@"Promise verification exception: %@",exception.reason);return false;}
    }
}
