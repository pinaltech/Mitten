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
                options.predicate = [NSPredicate predicateWithFormat:@"mediaType == %d || mediaType == %d",
                    (int)PHAssetMediaTypeImage, (int)PHAssetMediaTypeVideo];
                snapshot = [PHAsset fetchAssetsWithOptions:options];
            }
            NSMutableArray *items = [NSMutableArray new];
            NSUInteger end = MIN(snapshot.count, offset + limit);
            for (NSUInteger i = offset; i < end; ++i) {
                PHAsset *asset = [snapshot objectAtIndex:i];
                PHAssetResource *resource = [PHAssetResource assetResourcesForAsset:asset].firstObject;
                BOOL video = asset.mediaType == PHAssetMediaTypeVideo;
                NSMutableDictionary *item = [@{@"id": asset.localIdentifier,
                    @"name": resource.originalFilename ?: (video ? @"Video" : @"Photo"),
                    @"modified": @((asset.modificationDate ?: asset.creationDate).timeIntervalSince1970),
                    @"created": @(asset.creationDate.timeIntervalSince1970),
                    @"width": @(asset.pixelWidth), @"height": @(asset.pixelHeight), @"favorite": @(asset.favorite)} mutableCopy];
                if (video) item[@"duration"] = @(asset.duration);
                [items addObject:item];
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
        options.deliveryMode = PHImageRequestOptionsDeliveryModeHighQualityFormat;
        // Grid thumbnails need no exact scaling; previews keep it.
        options.resizeMode = edge <= 512 ? PHImageRequestOptionsResizeModeFast : PHImageRequestOptionsResizeModeExact;
        __block NSImage *result = nil;
        // Local derivatives first; only then fall back to downloading from iCloud.
        for (int network = 0; network < 2 && !result; ++network) {
            options.networkAccessAllowed = network == 1;
            [[PHImageManager defaultManager] requestImageForAsset:asset targetSize:CGSizeMake(edge, edge)
                contentMode:PHImageContentModeAspectFit options:options resultHandler:^(NSImage *image, NSDictionary *info) {
                    if (![info[PHImageResultIsDegradedKey] boolValue]) result = image;
                }];
        }
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
        PHAssetResourceType wanted = asset.mediaType == PHAssetMediaTypeVideo ? PHAssetResourceTypeVideo : PHAssetResourceTypePhoto;
        for (PHAssetResource *r in resources) if (r.type == wanted) { resource = r; break; }
        if (!resource) return failure(@"The original resource is unavailable.");
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
int dp_fill_square(CGImageRef image, unsigned edge, uint8_t *pixels);
// Small centre-cropped squares for dense grids, `count` at a time, straight from
// the thumbnails Photos already keeps (fast format, no iCloud download). One
// fetch resolves the whole batch. Fills `pixels` (count × edge² × 4, BGRA) and
// sets ok[i] for each square produced.
char *dp_photos_squares(const char **identifiers, size_t count, unsigned edge, uint8_t *pixels, uint8_t *ok) {
    @autoreleasepool {
        NSString *configurationError = usage_error();
        if (configurationError) return failure(configurationError);
        if (!authorized()) return failure(@"Photos permission is unavailable. Reconnect Apple Photos.");
        NSMutableArray<NSString *> *keys = [NSMutableArray arrayWithCapacity:count];
        for (size_t i = 0; i < count; ++i) [keys addObject:[NSString stringWithUTF8String:identifiers[i]]];
        NSMutableDictionary<NSString *, PHAsset *> *assets = [NSMutableDictionary dictionaryWithCapacity:count];
        [[PHAsset fetchAssetsWithLocalIdentifiers:keys options:nil] enumerateObjectsUsingBlock:^(PHAsset *asset, NSUInteger i, BOOL *stop) {
            (void)i; (void)stop;
            assets[asset.localIdentifier] = asset;
        }];
        PHImageRequestOptions *options = [PHImageRequestOptions new];
        options.synchronous = YES;
        options.deliveryMode = PHImageRequestOptionsDeliveryModeFastFormat;
        options.resizeMode = PHImageRequestOptionsResizeModeFast;
        options.networkAccessAllowed = NO;
        CGSize target = CGSizeMake(edge * 2, edge * 2);
        for (size_t i = 0; i < count; ++i) {
            ok[i] = 0;
            PHAsset *asset = assets[keys[i]];
            if (!asset) continue;
            __block NSImage *result = nil;
            [[PHImageManager defaultManager] requestImageForAsset:asset targetSize:target
                contentMode:PHImageContentModeAspectFill options:options resultHandler:^(NSImage *image, NSDictionary *info) {
                    (void)info;
                    if (image) result = image;
                }];
            if (!result) continue;
            CGImageRef cg = [result CGImageForProposedRect:NULL context:nil hints:nil];
            ok[i] = (uint8_t)dp_fill_square(cg, edge, pixels + i * (size_t)edge * edge * 4);
        }
        return reply(@{@"ok": @YES});
    }
}
// Every user album (including those inside folders) with its member asset
// identifiers. Smart albums are left out: favorites and videos have their own
// views, and the rest are derived from metadata Photos already has.
char *dp_photos_albums(void) {
    @autoreleasepool {
        NSString *configurationError = usage_error();
        if (configurationError) return failure(configurationError);
        if (!authorized()) return failure(@"Photos permission is unavailable. Reconnect Apple Photos.");
        NSMutableArray *albums = [NSMutableArray new];
        PHFetchResult<PHAssetCollection *> *collections =
            [PHAssetCollection fetchAssetCollectionsWithType:PHAssetCollectionTypeAlbum
                subtype:PHAssetCollectionSubtypeAlbumRegular options:nil];
        PHFetchOptions *members = [PHFetchOptions new];
        members.predicate = [NSPredicate predicateWithFormat:@"mediaType == %d || mediaType == %d",
            (int)PHAssetMediaTypeImage, (int)PHAssetMediaTypeVideo];
        for (PHAssetCollection *collection in collections) {
            @autoreleasepool {
                NSMutableArray *ids = [NSMutableArray new];
                for (PHAsset *asset in [PHAsset fetchAssetsInAssetCollection:collection options:members])
                    [ids addObject:asset.localIdentifier];
                [albums addObject:@{@"name": collection.localizedTitle ?: @"Untitled Album", @"ids": ids}];
            }
        }
        return reply(@{@"albums": albums});
    }
}
void dp_photos_free(char *value) { free(value); }
