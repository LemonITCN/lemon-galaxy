#import "CGVirtualDisplayBridge.h"
#import "CGVirtualDisplayPrivate.h"

#if !__has_feature(objc_arc)
#error "CGVirtualDisplayBridge 要用 ARC 编译（-fobjc-arc）"
#endif

BOOL LGVirtualDisplayAvailable(void) {
    Class display = NSClassFromString(@"CGVirtualDisplay");
    Class descriptor = NSClassFromString(@"CGVirtualDisplayDescriptor");
    Class settings = NSClassFromString(@"CGVirtualDisplaySettings");
    Class mode = NSClassFromString(@"CGVirtualDisplayMode");
    if (!display || !descriptor || !settings || !mode) return NO;
    return [display instancesRespondToSelector:@selector(initWithDescriptor:)]
        && [display instancesRespondToSelector:@selector(applySettings:)]
        && [display instancesRespondToSelector:@selector(displayID)]
        && [mode instancesRespondToSelector:@selector(initWithWidth:height:refreshRate:)]
        && [settings instancesRespondToSelector:@selector(setModes:)]
        && [descriptor instancesRespondToSelector:@selector(setMaxPixelsWide:)];
}

@implementation LGVirtualDisplay {
    CGVirtualDisplay *_display;
    CGDirectDisplayID _displayID;
}

- (nullable instancetype)initWithName:(NSString *)name
                             vendorID:(uint32_t)vendorID
                            productID:(uint32_t)productID
                         serialNumber:(uint32_t)serialNumber
                                width:(uint32_t)width
                               height:(uint32_t)height
                          refreshRate:(double)refreshRate
                                hiDPI:(BOOL)hiDPI
                                queue:(dispatch_queue_t)queue
                         onTerminated:(nullable void (^)(void))onTerminated
                                error:(NSString * _Nullable * _Nullable)error {
    self = [super init];
    if (!self) return nil;
    if (!LGVirtualDisplayAvailable()) {
        if (error) *error = @"这台 Mac 的系统不支持虚拟屏（没有 CGVirtualDisplay）";
        return nil;
    }
    if (width == 0 || height == 0 || refreshRate <= 0) {
        if (error) *error = [NSString stringWithFormat:@"不支持的分辨率 %ux%u@%.0f", width, height, refreshRate];
        return nil;
    }

    CGVirtualDisplayDescriptor *desc = [[NSClassFromString(@"CGVirtualDisplayDescriptor") alloc] init];
    if (!desc) {
        if (error) *error = @"无法创建虚拟屏描述（CGVirtualDisplayDescriptor）";
        return nil;
    }
    if ([desc respondsToSelector:@selector(setQueue:)]) {
        desc.queue = queue;
    } else if ([desc respondsToSelector:@selector(setDispatchQueue:)]) {
        [desc setDispatchQueue:queue];
    }
    desc.name = name;
    // 最大像素不小于要用的模式（HiDPI 时模式按点给，像素是两倍，见下面）
    desc.maxPixelsWide = width;
    desc.maxPixelsHigh = height;
    // 物理尺寸决定系统算出来的像素密度：普通屏按约 110 PPI，HiDPI 按约 220 PPI
    double ppi = hiDPI ? 220.0 : 110.0;
    desc.sizeInMillimeters = CGSizeMake(width / ppi * 25.4, height / ppi * 25.4);
    desc.vendorID = vendorID;
    desc.productID = productID;
    if ([desc respondsToSelector:@selector(setSerialNum:)]) {
        desc.serialNum = serialNumber;
    } else if ([desc respondsToSelector:NSSelectorFromString(@"setSerialNumber:")]) {
        [desc setValue:@(serialNumber) forKey:@"serialNumber"];
    }
    if (onTerminated && [desc respondsToSelector:@selector(setTerminationHandler:)]) {
        // 只捕获调用方的回调，不捕获 self（屏持有描述里的回调，self 持有屏，捕获 self 会成环）
        void (^cb)(void) = [onTerminated copy];
        desc.terminationHandler = ^(id _Nullable unknown, CGVirtualDisplay * _Nullable display) {
            (void)unknown;
            (void)display;
            cb();
        };
    }

    CGVirtualDisplay *display = [[NSClassFromString(@"CGVirtualDisplay") alloc] initWithDescriptor:desc];
    if (!display || display.displayID == kCGNullDirectDisplay) {
        if (error) *error = @"系统没能建出虚拟屏（CGVirtualDisplay 初始化失败；要在图形会话里的进程中建）";
        return nil;
    }

    CGVirtualDisplaySettings *settings = [[NSClassFromString(@"CGVirtualDisplaySettings") alloc] init];
    settings.hiDPI = hiDPI ? 1 : 0;
    // 只给一个模式：系统没有别的可选，开出来就是要求的分辨率与刷新率
    uint32_t mw = hiDPI ? width / 2 : width;
    uint32_t mh = hiDPI ? height / 2 : height;
    CGVirtualDisplayMode *mode = [[NSClassFromString(@"CGVirtualDisplayMode") alloc] initWithWidth:mw height:mh refreshRate:refreshRate];
    if (!mode) {
        if (error) *error = @"无法创建虚拟屏模式（CGVirtualDisplayMode）";
        return nil;
    }
    settings.modes = @[ mode ];
    if (![display applySettings:settings]) {
        if (error) *error = [NSString stringWithFormat:@"系统不接受虚拟屏模式 %ux%u@%.0f", width, height, refreshRate];
        return nil;
    }
    _display = display;
    _displayID = display.displayID;
    return self;
}

- (CGDirectDisplayID)displayID {
    @synchronized (self) {
        return _display ? _displayID : kCGNullDirectDisplay;
    }
}

- (void)invalidate {
    // 在锁外释放最后一个引用（系统随之移除这块屏，可能要一会儿）
    __attribute__((objc_precise_lifetime)) CGVirtualDisplay *d = nil;
    @synchronized (self) {
        d = _display;
        _display = nil;
    }
    (void)d;
}

@end
