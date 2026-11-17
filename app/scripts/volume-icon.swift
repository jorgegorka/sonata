import CoreGraphics
import Foundation
import ImageIO

let args = CommandLine.arguments
guard args.count == 3,
    let source = CGImageSourceCreateWithURL(URL(fileURLWithPath: args[1]) as CFURL, nil),
    let icon = CGImageSourceCreateImageAtIndex(source, 0, nil)
else {
    FileHandle.standardError.write(Data("usage: swift volume-icon.swift <1024x1024 png> <out.iconset>\n".utf8))
    exit(1)
}
let out = URL(fileURLWithPath: args[2])
try FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)

func body(_ side: Double) -> CGPath {
    let unit = side / 1024
    let rect = CGRect(x: 100 * unit, y: 100 * unit, width: 824 * unit, height: 824 * unit)
    let radius = 220 * unit
    let curves: [(CGPoint, CGPoint, CGPoint)] = [
        (CGPoint(x: 1.08849296, y: 0), CGPoint(x: 0.86840694, y: 0.02239799), CGPoint(x: 0.63149399, y: 0.07491100)),
        (CGPoint(x: 0.37282392, y: 0.13385525), CGPoint(x: 0.13385525, y: 0.37282392), CGPoint(x: 0.07491100, y: 0.63149399)),
        (CGPoint(x: 0.02239799, y: 0.86840694), CGPoint(x: 0, y: 1.08849296), CGPoint(x: 0, y: 1.52866483)),
    ]
    let corners: [(CGPoint, CGVector, CGVector)] = [
        (CGPoint(x: rect.minX, y: rect.maxY), CGVector(dx: 0, dy: -1), CGVector(dx: 1, dy: 0)),
        (CGPoint(x: rect.maxX, y: rect.maxY), CGVector(dx: -1, dy: 0), CGVector(dx: 0, dy: -1)),
        (CGPoint(x: rect.maxX, y: rect.minY), CGVector(dx: 0, dy: 1), CGVector(dx: -1, dy: 0)),
        (CGPoint(x: rect.minX, y: rect.minY), CGVector(dx: 1, dy: 0), CGVector(dx: 0, dy: 1)),
    ]
    let path = CGMutablePath()
    for (index, (point, back, next)) in corners.enumerated() {
        func at(_ q: CGPoint) -> CGPoint {
            CGPoint(
                x: point.x + (q.x * back.dx + q.y * next.dx) * radius,
                y: point.y + (q.x * back.dy + q.y * next.dy) * radius)
        }
        let start = at(CGPoint(x: 1.52866483, y: 0))
        if index == 0 { path.move(to: start) } else { path.addLine(to: start) }
        for (control1, control2, end) in curves {
            path.addCurve(to: at(end), control1: at(control1), control2: at(control2))
        }
    }
    path.closeSubpath()
    return path
}

for points in [16, 32, 128, 256, 512] {
    for scale in [1, 2] {
        let side = points * scale
        let context = CGContext(
            data: nil, width: side, height: side, bitsPerComponent: 8, bytesPerRow: 0,
            space: CGColorSpace(name: CGColorSpace.sRGB)!,
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
        context.interpolationQuality = .high
        let s = Double(side)
        context.addPath(body(s))
        context.clip()
        context.draw(icon, in: CGRect(x: s * 100 / 1024, y: s * 100 / 1024, width: s * 824 / 1024, height: s * 824 / 1024))
        let name = "icon_\(points)x\(points)\(scale == 2 ? "@2x" : "").png"
        let file = CGImageDestinationCreateWithURL(out.appendingPathComponent(name) as CFURL, "public.png" as CFString, 1, nil)!
        CGImageDestinationAddImage(file, context.makeImage()!, nil)
        guard CGImageDestinationFinalize(file) else { fatalError("could not write \(name)") }
    }
}
