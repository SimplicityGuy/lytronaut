// MIT independently authored synthetic compatibility verifier, macOS test-only.
// Uses resource loaders; never opens Photos, PHPhotoLibrary, or an iCloud library.
// Compile with a task-specific /tmp -module-cache-path; pass example output directory.
import Foundation
import AppKit
import AVFoundation
import ImageIO
import Photos

let directory = URL(fileURLWithPath: CommandLine.arguments[1], isDirectory: true)
let jpeg = directory.appendingPathComponent("focus.JPG")
let movie = directory.appendingPathComponent("focus.MOV")
let manifest = try JSONSerialization.jsonObject(with: Data(contentsOf: directory.appendingPathComponent("provenance.json"))) as! [String: Any]
precondition(manifest["fixture"] as? String == "independent_mit_rgb")
let expected = manifest["playback_rgb"] as! [[Int]]
let selected = manifest["selected_rgb"] as! [Int]
let keySeconds = manifest["key_seconds"] as! Double
func matches(_ actual: [Int], _ expected: [Int]) -> Bool {
    zip(actual, expected).allSatisfy { abs($0 - $1) <= 3 }
}
let source = CGImageSourceCreateWithURL(jpeg as CFURL, nil)!
let image = CGImageSourceCreateImageAtIndex(source, 0, nil)!
let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil)! as NSDictionary
precondition((properties[kCGImagePropertyOrientation] as? Int) == 1)
var still = [UInt8](repeating: 0, count: image.width * image.height * 4)
let stillContext = CGContext(data: &still, width: image.width, height: image.height,
    bitsPerComponent: 8, bytesPerRow: image.width * 4, space: CGColorSpaceCreateDeviceRGB(),
    bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)!
stillContext.draw(image, in: CGRect(x: 0, y: 0, width: image.width, height: image.height))
let center = (image.height / 2 * image.width + image.width / 2) * 4
precondition(matches(still[center..<center+3].map(Int.init), selected))
var completed = false
var failed = false
Task {
    do {
        let asset = AVURLAsset(url: movie)
        let video = try await asset.loadTracks(withMediaType: .video)
        precondition(video.count == 1)
        let audio = try await asset.loadTracks(withMediaType: .audio)
        precondition(audio.isEmpty)
        let duration = try await asset.load(.duration)
        precondition(abs(CMTimeGetSeconds(duration) - 3.0) < 0.001)
        let reader = try AVAssetReader(asset: asset)
        let output = AVAssetReaderTrackOutput(track: video[0], outputSettings: [
            kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_32BGRA])
        reader.add(output)
        precondition(reader.startReading())
        var pixels = [[Int]]()
        while let sample = output.copyNextSampleBuffer() {
            let buffer = CMSampleBufferGetImageBuffer(sample)!
            precondition(CVPixelBufferGetWidth(buffer) == image.width && CVPixelBufferGetHeight(buffer) == image.height)
            CVPixelBufferLockBaseAddress(buffer, .readOnly)
            let bytes = CVPixelBufferGetBaseAddress(buffer)!.assumingMemoryBound(to: UInt8.self)
            let offset = CVPixelBufferGetBytesPerRow(buffer) * (image.height / 2) + (image.width / 2) * 4
            pixels.append([Int(bytes[offset+2]), Int(bytes[offset+1]), Int(bytes[offset])])
            CVPixelBufferUnlockBaseAddress(buffer, .readOnly)
        }
        precondition(reader.status == .completed && pixels.count == expected.count)
        precondition(zip(pixels, expected).allSatisfy { matches($0, $1) })
        let metadata = try await asset.load(.metadata)
        let identifiers = metadata.filter { $0.identifier == .quickTimeMetadataContentIdentifier }
        precondition(identifiers.count == 1)
        let id = try await identifiers[0].load(.stringValue)!
        let jpegBytes = try Data(contentsOf: jpeg)
        precondition(jpegBytes.range(of: Data(id.utf8)) != nil)
        let tracks = try await asset.loadTracks(withMediaType: .metadata)
        precondition(tracks.count == 1)
        let metadataReader = try AVAssetReader(asset: asset)
        let metadataOutput = AVAssetReaderTrackOutput(track: tracks[0], outputSettings: nil)
        let adaptor = AVAssetReaderOutputMetadataAdaptor(assetReaderTrackOutput: metadataOutput)
        metadataReader.add(metadataOutput)
        precondition(metadataReader.startReading())
        let group = adaptor.nextTimedMetadataGroup()!
        precondition(abs(CMTimeGetSeconds(group.timeRange.start) - keySeconds) < 0.001)
        precondition(group.items.count == 1 && group.items[0].identifier?.rawValue == "mdta/com.apple.quicktime.still-image-time")
        let value = try await group.items[0].load(.numberValue)
        precondition(value?.intValue == -1 && adaptor.nextTimedMetadataGroup() == nil)
        print("appleDecodedFrames=\(pixels.count) selectedStillCorrect=true keyTimeCorrect=true noAudio=true")
    } catch { failed = true }
    completed = true
}
let decodeDeadline = Date(timeIntervalSinceNow: 20)
while !completed && Date() < decodeDeadline { RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.05)) }
precondition(completed && !failed)
func recognize(_ resources: [URL]) -> Bool {
    var done = false
    var recognized = false
    PHLivePhoto.request(withResourceFileURLs: resources, placeholderImage: nil,
        targetSize: CGSize(width: image.width, height: image.height), contentMode: .aspectFit) { photo, info in
        if !(info[PHLivePhotoInfoIsDegradedKey] as? Bool ?? false) {
            recognized = photo != nil && info[PHLivePhotoInfoErrorKey] == nil
            done = true
        }
    }
    let deadline = Date(timeIntervalSinceNow: 20)
    while !done && Date() < deadline { RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.05)) }
    precondition(done)
    return recognized
}
precondition(recognize([jpeg, movie]))
let negative = directory.appendingPathComponent("mismatched-test-only.JPG")
defer { try? FileManager.default.removeItem(at: negative) }
var bytes = try Data(contentsOf: jpeg)
let marker = Data("A8A8A8A8-1111-4444-8888-121212121212".utf8)
let range = bytes.range(of: marker)!
bytes.replaceSubrange(range, with: Data("B9B9B9B9-2222-4444-8888-232323232323".utf8))
try bytes.write(to: negative, options: .atomic)
precondition(!recognize([negative, movie]))
print("livePhotoResourceRecognized=true mismatchedPairRejected=true photosLibraryOpened=false guiImportAcceptancePending=true")
