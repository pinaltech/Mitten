import AppKit

// Draws the Mitten logo (assets/brand/mitten.svg) on a light app-icon tile.
let output = CommandLine.arguments[1]
try FileManager.default.createDirectory(atPath: output, withIntermediateDirectories: true)

func color(_ hex: UInt32) -> CGColor {
    CGColor(srgbRed: CGFloat((hex >> 16) & 0xff) / 255, green: CGFloat((hex >> 8) & 0xff) / 255, blue: CGFloat(hex & 0xff) / 255, alpha: 1)
}
func gradient(_ stops: [(CGFloat, UInt32)]) -> CGGradient {
    CGGradient(colorsSpace: CGColorSpace(name: CGColorSpace.sRGB), colors: stops.map { color($0.1) } as CFArray, locations: stops.map { $0.0 })!
}
func point(_ x: CGFloat, _ y: CGFloat) -> CGPoint { CGPoint(x: x, y: y) }
// Paths in the SVG's 640×540 coordinate space, y pointing down.
let butte = CGMutablePath()
butte.move(to: point(40, 396))
butte.addLine(to: point(40, 352))
butte.addLine(to: point(192, 240))
butte.addLine(to: point(198, 120))
butte.addQuadCurve(to: point(213, 105), control: point(199, 106))
butte.addLine(to: point(294, 96))
butte.addLine(to: point(362, 102))
butte.addQuadCurve(to: point(376, 116), control: point(375, 103))
butte.addLine(to: point(380, 200))
butte.addQuadCurve(to: point(391, 210), control: point(381, 210))
butte.addLine(to: point(403, 210))
butte.addQuadCurve(to: point(414, 200), control: point(413, 210))
butte.addLine(to: point(417, 138))
butte.addQuadCurve(to: point(431, 124), control: point(418, 124))
butte.addLine(to: point(439, 124))
butte.addQuadCurve(to: point(453, 138), control: point(452, 124))
butte.addLine(to: point(460, 240))
butte.addLine(to: point(600, 352))
butte.addLine(to: point(600, 396))
butte.addQuadCurve(to: point(592, 397), control: point(600, 403))
butte.addCurve(to: point(432, 327), control1: point(550, 362), control2: point(493, 339))
butte.addQuadCurve(to: point(208, 327), control: point(320, 306))
butte.addCurve(to: point(48, 397), control1: point(147, 339), control2: point(90, 362))
butte.addQuadCurve(to: point(40, 396), control: point(40, 403))
butte.closeSubpath()
// The lens ring is the even-odd gap between these two circles.
butte.addEllipse(in: CGRect(x: 227, y: 128, width: 120, height: 120))
butte.addEllipse(in: CGRect(x: 253, y: 154, width: 68, height: 68))

for (name, pixels) in [("icon_16x16",16),("icon_16x16@2x",32),("icon_32x32",32),("icon_32x32@2x",64),("icon_128x128",128),("icon_128x128@2x",256),("icon_256x256",256),("icon_256x256@2x",512),("icon_512x512",512),("icon_512x512@2x",1024)] {
    let bitmap = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: pixels, pixelsHigh: pixels, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: bitmap)
    let context = NSGraphicsContext.current!.cgContext
    context.scaleBy(x: CGFloat(pixels) / 1024, y: CGFloat(pixels) / 1024)
    // Tile, with the inset and corner radius of macOS app icons.
    let tile = CGPath(roundedRect: CGRect(x: 100, y: 100, width: 824, height: 824), cornerWidth: 185, cornerHeight: 185, transform: nil)
    context.saveGState()
    context.setShadow(offset: CGSize(width: 0, height: -10), blur: 24, color: CGColor(gray: 0, alpha: 0.28))
    context.addPath(tile)
    context.setFillColor(color(0xffffff))
    context.fillPath()
    context.restoreGState()
    context.saveGState()
    context.addPath(tile)
    context.clip()
    context.drawLinearGradient(gradient([(0, 0xffffff), (1, 0xe6f2ea)]), start: CGPoint(x: 0, y: 924), end: CGPoint(x: 0, y: 100), options: [])
    context.restoreGState()
    // Logo: flip to SVG coordinates and center the artwork on the tile.
    context.translateBy(x: 0, y: 1024)
    context.scaleBy(x: 1, y: -1)
    context.translateBy(x: 512 - 320, y: 512 - 298)
    context.saveGState()
    context.addPath(butte)
    context.clip(using: .evenOdd)
    context.drawLinearGradient(gradient([(0, 0x35cd58), (0.55, 0x18b965), (1, 0x008d77)]), start: point(40, 60), end: point(600, 400), options: [])
    context.restoreGState()
    context.saveGState()
    context.addEllipse(in: CGRect(x: 40, y: 358, width: 560, height: 284))
    context.clip()
    context.clip(to: CGRect(x: 0, y: 0, width: 640, height: 500))
    context.drawLinearGradient(gradient([(0, 0x003d40), (1, 0x005353)]), start: point(40, 0), end: point(600, 0), options: [])
    context.restoreGState()
    NSGraphicsContext.restoreGraphicsState()
    try bitmap.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: "\(output)/\(name).png"))
}
