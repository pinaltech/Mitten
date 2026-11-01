#import <AppKit/AppKit.h>
#import <objc/runtime.h>
#include <stdlib.h>
#include <stdio.h>
static void DPTrace(const char *message) { if (getenv("DESERT_PHOTOS_TRACE_DRAGS")) fprintf(stderr,"Photos drag: %s\n",message); }
#import <UniformTypeIdentifiers/UniformTypeIdentifiers.h>

typedef bool (*DPWriteOriginal)(void *, size_t, const char *, char *, size_t);
typedef void (*DPReleaseContext)(void *);

@interface DPDragContext : NSObject
@property(nonatomic) void *rust;
@property(nonatomic) DPWriteOriginal write;
@property(nonatomic) DPReleaseContext releaseContext;
@end
@implementation DPDragContext
- (void)dealloc { if (_rust && _releaseContext) _releaseContext(_rust); }
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
@interface DPFilePromise : NSFilePromiseProvider
@property(nonatomic, strong) DPFileWriter *writer;
@property(nonatomic, copy) NSString *localPath;
@end
@implementation DPFilePromise
- (NSArray<NSPasteboardType> *)writableTypesForPasteboard:(NSPasteboard *)pasteboard {
    NSMutableArray *types = [[super writableTypesForPasteboard:pasteboard] mutableCopy];
    if (self.localPath) [types addObjectsFromArray:@[NSPasteboardTypeFileURL]];
    return types;
}
- (id)pasteboardPropertyListForType:(NSPasteboardType)type {
    if (self.localPath && [type isEqualToString:NSPasteboardTypeFileURL])
        return [NSURL fileURLWithPath:self.localPath].absoluteString;
    return [super pasteboardPropertyListForType:type];
}
@end

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
        NSData *data = [[NSString stringWithUTF8String:json] dataUsingEncoding:NSUTF8StringEncoding];
        NSArray *records = [NSJSONSerialization JSONObjectWithData:data options:0 error:NULL];
        if (![records isKindOfClass:[NSArray class]] || records.count == 0) return false;
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
