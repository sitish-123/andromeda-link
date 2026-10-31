package org.andromeda.link.core.data

import android.content.Context
import android.graphics.Bitmap
import android.graphics.ImageDecoder
import android.media.ThumbnailUtils
import android.provider.MediaStore
import android.util.Size
import org.andromeda.link.ffi.LibraryItem
import org.andromeda.link.ffi.PhoneLibrary
import java.io.ByteArrayOutputStream
import java.io.File
import kotlin.math.roundToInt

/**
 * The media store by album, for a desktop browsing `Photos`: images and videos, which the core reads by path (the
 * media permissions allow that), and PNG thumbnails of them. Called off the main thread.
 */
class MediaLibrary(
    private val context: Context,
) : PhoneLibrary {
    /** Reads only the album column: on a large library the full rows took a slow phone past the desktop's timeout. */
    override fun albums(): List<String> {
        val type = MediaStore.Files.FileColumns.MEDIA_TYPE
        val args =
            arrayOf(
                MediaStore.Files.FileColumns.MEDIA_TYPE_IMAGE
                    .toString(),
                MediaStore.Files.FileColumns.MEDIA_TYPE_VIDEO
                    .toString(),
            )
        val collection = MediaStore.Files.getContentUri(MediaStore.VOLUME_EXTERNAL)
        return context.contentResolver
            .query(collection, arrayOf(MediaStore.MediaColumns.BUCKET_DISPLAY_NAME), "($type=? OR $type=?)", args, null)
            ?.use { cursor -> buildSet { while (cursor.moveToNext()) add(cursor.getString(0) ?: "Other") } }
            .orEmpty()
            .sorted()
    }

    override fun album(name: String): List<LibraryItem> = query(name).map { it.item }

    override fun thumbnail(
        path: String,
        px: UShort,
    ): ByteArray? =
        runCatching {
            val file = File(path)
            val bitmap =
                if (mimeTypeOf(file.name).startsWith("video/")) {
                    ThumbnailUtils.createVideoThumbnail(file, Size(px.toInt(), px.toInt()), null)
                } else {
                    imageThumbnail(file, px.toInt())
                }
            ByteArrayOutputStream()
                .also { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
                .toByteArray()
                .also { bitmap.recycle() }
        }.getOrNull()

    /**
     * The image scaled to [px] on its longer side, upright. ThumbnailUtils returns the small embedded EXIF thumbnail
     * when there is one, and its whole-number subsampling leaves the shorter side of a tall screenshot near px / 2.
     * ImageDecoder applies the EXIF orientation itself.
     */
    private fun imageThumbnail(
        file: File,
        px: Int,
    ): Bitmap =
        ImageDecoder.decodeBitmap(ImageDecoder.createSource(file)) { decoder, info, _ ->
            val (width, height) = info.size.width to info.size.height
            val scale = minOf(1f, px.toFloat() / maxOf(width, height))
            decoder.setTargetSize(maxOf(1, (width * scale).roundToInt()), maxOf(1, (height * scale).roundToInt()))
            decoder.allocator = ImageDecoder.ALLOCATOR_SOFTWARE
        }

    private class Row(
        val album: String,
        val item: LibraryItem,
    )

    private fun query(album: String?): List<Row> {
        val type = MediaStore.Files.FileColumns.MEDIA_TYPE
        val bucket = MediaStore.MediaColumns.BUCKET_DISPLAY_NAME
        val selection = "($type=? OR $type=?)" + if (album != null) " AND $bucket=?" else ""
        val args =
            listOfNotNull(
                MediaStore.Files.FileColumns.MEDIA_TYPE_IMAGE
                    .toString(),
                MediaStore.Files.FileColumns.MEDIA_TYPE_VIDEO
                    .toString(),
                album,
            ).toTypedArray()
        val projection =
            arrayOf(
                bucket,
                MediaStore.MediaColumns.DISPLAY_NAME,
                @Suppress("DEPRECATION") MediaStore.MediaColumns.DATA,
                MediaStore.MediaColumns.SIZE,
                MediaStore.MediaColumns.DATE_MODIFIED,
            )
        val collection = MediaStore.Files.getContentUri(MediaStore.VOLUME_EXTERNAL)
        return context.contentResolver
            .query(collection, projection, selection, args, null)
            ?.use { cursor ->
                buildList {
                    while (cursor.moveToNext()) {
                        val name = cursor.getString(1) ?: continue
                        val path = cursor.getString(2) ?: continue
                        val item = LibraryItem(name, path, cursor.getLong(3).toULong(), cursor.getLong(4).toULong())
                        add(Row(cursor.getString(0) ?: "Other", item))
                    }
                }
            }.orEmpty()
    }
}
