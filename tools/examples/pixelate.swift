// Usage: swift tools/examples/pixelate.swift <blocks-across> <in.jpg> <out.jpg>
// Pixelates a whole photo into coarse blocks, so published art in an example game can't be
// recognised but its shapes and colours still show. Writes a JPEG with no camera metadata.
import CoreImage
import Foundation

let a = CommandLine.arguments
guard a.count == 4, let blocks = Double(a[1]), let img = CIImage(contentsOf: URL(fileURLWithPath: a[2])) else {
    FileHandle.standardError.write("usage: pixelate.swift <blocks-across> <in> <out>\n".data(using: .utf8)!)
    exit(2)
}
let e = img.extent
let f = CIFilter(name: "CIPixellate")!
f.setValue(img.clampedToExtent(), forKey: kCIInputImageKey)
f.setValue(e.width / blocks, forKey: kCIInputScaleKey)
f.setValue(CIVector(x: e.midX, y: e.midY), forKey: kCIInputCenterKey)
let out = f.outputImage!.cropped(to: e)
let ctx = CIContext()
try ctx.writeJPEGRepresentation(of: out, to: URL(fileURLWithPath: a[3]),
    colorSpace: CGColorSpace(name: CGColorSpace.sRGB)!,
    options: [kCGImageDestinationLossyCompressionQuality as CIImageRepresentationOption: 0.85])
