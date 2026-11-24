// CoreGraphics 私有类 CGVirtualDisplay 的接口声明（公开 SDK 里没有头文件）。
//
// 来源：照 DeskPad（https://github.com/Stengo/DeskPad ，MIT 许可）的 CGVirtualDisplayPrivate.h 与
// Chromium 测试代码（ui/display/mac/test/virtual_display_mac_util.mm）里的用法写；
// 各成员的类型按 macOS 27.2.1 运行时用 class_copyPropertyList / class_copyMethodList 读到的类型编码核对过
// （例如 CGVirtualDisplayMode 的宽高是 unsigned int，initWithWidth:height:refreshRate: 的编码为 @32@0:8I16I20d24）。
// 只声明本工程用到的成员。
//
// 这些类只用 NSClassFromString 在运行时取，代码里不直接写类名发消息，所以不产生静态链接引用：
// 哪个系统版本去掉了它们，应用照常启动，只是虚拟屏报不支持（见 LGVirtualDisplayAvailable）。

#import <Foundation/Foundation.h>
#import <CoreGraphics/CoreGraphics.h>

NS_ASSUME_NONNULL_BEGIN

@class CGVirtualDisplay;

@interface CGVirtualDisplayDescriptor : NSObject
/// 终止回调在这个队列上调用（老系统上只有 setDispatchQueue:）
@property (retain, nonatomic) dispatch_queue_t queue;
@property (retain, nonatomic) NSString *name;
@property (nonatomic) unsigned int maxPixelsWide;
@property (nonatomic) unsigned int maxPixelsHigh;
@property (nonatomic) CGSize sizeInMillimeters;
/// macOS 14 起不同的虚拟屏要用不同的序列号（Chromium 源码注释）；新系统另有同义的 serialNumber
@property (nonatomic) unsigned int serialNum;
@property (nonatomic) unsigned int productID;
@property (nonatomic) unsigned int vendorID;
/// 系统结束这块屏时调用（例如窗口服务器重启）
@property (copy, nonatomic, nullable) void (^terminationHandler)(id _Nullable unknown, CGVirtualDisplay * _Nullable display);
- (instancetype)init;
- (void)setDispatchQueue:(dispatch_queue_t)queue;
@end

@interface CGVirtualDisplayMode : NSObject
@property (readonly, nonatomic) unsigned int width;
@property (readonly, nonatomic) unsigned int height;
@property (readonly, nonatomic) double refreshRate;
- (instancetype)initWithWidth:(unsigned int)width height:(unsigned int)height refreshRate:(double)refreshRate;
@end

@interface CGVirtualDisplaySettings : NSObject
@property (retain, nonatomic) NSArray<CGVirtualDisplayMode *> *modes;
/// 1 = HiDPI（模式里的宽高按「点」算，像素是两倍）
@property (nonatomic) unsigned int hiDPI;
- (instancetype)init;
@end

@interface CGVirtualDisplay : NSObject
@property (readonly, nonatomic) CGDirectDisplayID displayID;
- (nullable instancetype)initWithDescriptor:(CGVirtualDisplayDescriptor *)descriptor;
- (BOOL)applySettings:(CGVirtualDisplaySettings *)settings;
@end

NS_ASSUME_NONNULL_END
