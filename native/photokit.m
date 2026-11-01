#import <Photos/Photos.h>
#import <AppKit/AppKit.h>

// Called exclusively by background workers. A fetch result is a lazy PhotoKit
// snapshot: enumerating a page never loads image data for the rest of the library.
static PHFetchResult<PHAsset *> *snapshot;
static char *reply(NSDictionary *result) {
    NSData *data = [NSJSONSerialization dataWithJSONObject:result options:0 error:nil];
    return strdup([[NSString alloc] initWithData:data encoding:NSUTF8StringEncoding].UTF8String);
}
static char *failure(NSString *message) { return reply(@{@"error": message ?: @"PhotoKit request failed"}); }
// Check before any PhotoKit authorization API: TCC aborts the process rather
// than returning an NSError when the usage description is missing.
static NSString *usage_error(void) {
    id usage = [[NSBundle mainBundle] objectForInfoDictionaryKey:@"NSPhotoLibraryUsageDescription"];
    if (![usage isKindOfClass:[NSString class]] ||
        [[usage stringByTrimmingCharactersInSet:[NSCharacterSet whitespaceAndNewlineCharacterSet]] length] == 0) {
        return @"This build is missing NSPhotoLibraryUsageDescription. Rebuild Photos with scripts/bundle-macos.sh before connecting Apple Photos.";
    }
    return nil;
}
char *dp_photos_validate_usage(void) {
    @autoreleasepool {
        NSString *error = usage_error();
        return error ? failure(error) : reply(@{@"ok": @YES});
    }
}
static BOOL authorized(void) {
    PHAuthorizationStatus status = [PHPhotoLibrary authorizationStatusForAccessLevel:PHAccessLevelReadWrite];
    if (status == PHAuthorizationStatusNotDetermined) {
        dispatch_semaphore_t done = dispatch_semaphore_create(0);
        [PHPhotoLibrary requestAuthorizationForAccessLevel:PHAccessLevelReadWrite handler:^(PHAuthorizationStatus s) {
            (void)s;
            dispatch_semaphore_signal(done);
        }];
        dispatch_semaphore_wait(done, DISPATCH_TIME_FOREVER);
        status = [PHPhotoLibrary authorizationStatusForAccessLevel:PHAccessLevelReadWrite];
    }
    return status == PHAuthorizationStatusAuthorized || status == PHAuthorizationStatusLimited;
}
char *dp_photos_page(size_t offset, size_t limit) {
    @autoreleasepool {
        NSString *configurationError = usage_error();
        if (configurationError) return failure(configurationError);
        if (!authorized()) return failure(@"Allow Photos access in System Settings → Privacy & Security → Photos, then reconnect.");
        @synchronized([PHPhotoLibrary class]) {
            if (offset == 0 || snapshot == nil) {
                PHFetchOptions *options = [PHFetchOptions new];
                options.sortDescriptors = @[[NSSortDescriptor sortDescriptorWithKey:@"creationDate" ascending:NO]];
                snapshot = [PHAsset fetchAssetsWithMediaType:PHAssetMediaTypeImage options:options];
            }
            NSMutableArray *items = [NSMutableArray new];
            NSUInteger end = MIN(snapshot.count, offset + limit);
            for (NSUInteger i = offset; i < end; ++i) {
                PHAsset *asset = [snapshot objectAtIndex:i];
                PHAssetResource *resource = [PHAssetResource assetResourcesForAsset:asset].firstObject;
                [items addObject:@{@"id": asset.localIdentifier, @"name": resource.originalFilename ?: @"Photo",
                    @"modified": @((asset.modificationDate ?: asset.creationDate).timeIntervalSince1970),
                    @"created": @(asset.creationDate.timeIntervalSince1970),
                    @"width": @(asset.pixelWidth), @"height": @(asset.pixelHeight), @"favorite": @(asset.favorite)}];
            }
            return reply(@{@"items": items, @"total": @(snapshot.count), @"end": @(end)});
        }
    }
}
char *dp_photos_image(const char *identifier, const char *destination, unsigned edge) {
    @autoreleasepool {
        NSString *configurationError = usage_error();
        if (configurationError) return failure(configurationError);
        if (!authorized()) return failure(@"Photos permission is unavailable. Reconnect Apple Photos.");
        NSString *key = [NSString stringWithUTF8String:identifier];
        PHAsset *asset = [PHAsset fetchAssetsWithLocalIdentifiers:@[key] options:nil].firstObject;
        if (!asset) return failure(@"This photo is no longer available.");
        PHImageRequestOptions *options = [PHImageRequestOptions new];
        options.synchronous = YES;
        options.networkAccessAllowed = YES;
        options.deliveryMode = PHImageRequestOptionsDeliveryModeHighQualityFormat;
        options.resizeMode = PHImageRequestOptionsResizeModeExact;
        __block NSImage *result = nil;
        [[PHImageManager defaultManager] requestImageForAsset:asset targetSize:CGSizeMake(edge, edge)
            contentMode:PHImageContentModeAspectFit options:options resultHandler:^(NSImage *image, NSDictionary *info) {
                if (![info[PHImageResultIsDegradedKey] boolValue]) result = image;
            }];
        if (!result) return failure(@"PhotoKit could not load this preview. Check iCloud connectivity.");
        CGImageRef cg = [result CGImageForProposedRect:NULL context:nil hints:nil];
        if (!cg) return failure(@"PhotoKit returned an unsupported image.");
        NSBitmapImageRep *bitmap = [[NSBitmapImageRep alloc] initWithCGImage:cg];
        NSData *data = [bitmap representationUsingType:NSBitmapImageFileTypeJPEG properties:@{NSImageCompressionFactor:@0.88}];
        NSError *error = nil;
        if (![data writeToFile:[NSString stringWithUTF8String:destination] options:NSDataWritingAtomic error:&error]) return failure(error.localizedDescription);
        return reply(@{@"ok": @YES});
    }
}
char *dp_photos_original(const char *identifier, const char *destination) {
    @autoreleasepool {
        NSString *configurationError = usage_error();
        if (configurationError) return failure(configurationError);
        if (!authorized()) return failure(@"Photos permission is unavailable.");
        PHAsset *asset = [PHAsset fetchAssetsWithLocalIdentifiers:@[[NSString stringWithUTF8String:identifier]] options:nil].firstObject;
        if (!asset) return failure(@"This photo is no longer available.");
        NSArray<PHAssetResource *> *resources = [PHAssetResource assetResourcesForAsset:asset];
        PHAssetResource *resource = nil;
        for (PHAssetResource *r in resources) if (r.type == PHAssetResourceTypePhoto) { resource = r; break; }
        if (!resource) return failure(@"The original image resource is unavailable.");
        PHAssetResourceRequestOptions *options = [PHAssetResourceRequestOptions new];
        options.networkAccessAllowed = YES;
        dispatch_semaphore_t done = dispatch_semaphore_create(0);
        __block NSError *problem = nil;
        // requestData streams the resource without retaining a full-resolution image.
        NSFileHandle *file = [NSFileHandle fileHandleForWritingAtPath:[NSString stringWithUTF8String:destination]];
        if (!file) return failure(@"Cannot create export file.");
        [[PHAssetResourceManager defaultManager] requestDataForAssetResource:resource options:options dataReceivedHandler:^(NSData *data) {
            NSError *error = nil;
            if (![file writeData:data error:&error]) problem = error;
        } completionHandler:^(NSError *error) { if (error) problem = error; dispatch_semaphore_signal(done); }];
        dispatch_semaphore_wait(done, DISPATCH_TIME_FOREVER);
        [file closeFile];
        if (problem) return failure(problem.localizedDescription);
        return reply(@{@"name": resource.originalFilename});
    }
}
void dp_photos_free(char *value) { free(value); }
