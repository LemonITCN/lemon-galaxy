// 虚拟屏（CGVirtualDisplay 私有接口）的薄封装，给 Swift 用。说明见 Platform/VirtualDisplays.swift
// 与 docs/research/virtual-display.md 第 4 节。

#import <Foundation/Foundation.h>
#import <CoreGraphics/CoreGraphics.h>

NS_ASSUME_NONNULL_BEGIN

/// 本机系统有没有可用的 CGVirtualDisplay（运行时探测类与方法是否都在；macOS 11 起有）。
BOOL LGVirtualDisplayAvailable(void);

/// 一块虚拟屏。对象活着屏就在；释放或 invalidate 即删屏（系统异步移除，约 1 秒内从屏幕列表消失），进程退出时系统自动回收。
/// 建屏要在连着窗口服务器的进程里做（图形会话里的进程；launchd 守护进程不行）。
@interface LGVirtualDisplay : NSObject

- (instancetype)init NS_UNAVAILABLE;

/// 建一块虚拟屏并设成指定模式。width / height 是像素；hiDPI 为 YES 时按 2 倍缩放（看起来是一半大小的点）。
/// onTerminated 在 queue 上调用（系统自己结束了这块屏）。失败返回 nil，error 里是原因。
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
                                error:(NSString * _Nullable * _Nullable)error NS_DESIGNATED_INITIALIZER;

/// 系统分配的屏幕号（与 CGGetActiveDisplayList、ScreenCaptureKit 的 SCDisplay.displayID 相同）；invalidate 后为 0。
@property (nonatomic, readonly) CGDirectDisplayID displayID;

/// 立即删屏，可重复调用。
- (void)invalidate;

@end

NS_ASSUME_NONNULL_END
