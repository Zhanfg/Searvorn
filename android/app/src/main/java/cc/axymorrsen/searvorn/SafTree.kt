package cc.axymorrsen.searvorn

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.provider.DocumentsContract

internal data class SafTreeSnapshot(
    val rootName: String,
    val entries: Int,
    val truncated: Boolean,
)

internal class SafTree(private val context: Context) {
    fun pickerIntent(): Intent =
        Intent(Intent.ACTION_OPEN_DOCUMENT_TREE).apply {
            addFlags(
                Intent.FLAG_GRANT_READ_URI_PERMISSION or
                    Intent.FLAG_GRANT_WRITE_URI_PERMISSION or
                    Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION or
                    Intent.FLAG_GRANT_PREFIX_URI_PERMISSION,
            )
        }

    fun persist(uri: Uri, resultFlags: Int) {
        val takeFlags =
            resultFlags and
                (Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION)
        if (takeFlags != 0) {
            context.contentResolver.takePersistableUriPermission(uri, takeFlags)
        }
    }

    fun snapshot(treeUri: Uri, maxEntries: Int = 256): SafTreeSnapshot {
        require(maxEntries > 0)

        val documentId = DocumentsContract.getTreeDocumentId(treeUri)
        val rootUri = DocumentsContract.buildDocumentUriUsingTree(treeUri, documentId)
        val childrenUri = DocumentsContract.buildChildDocumentsUriUsingTree(treeUri, documentId)
        val projection =
            arrayOf(
                DocumentsContract.Document.COLUMN_DOCUMENT_ID,
                DocumentsContract.Document.COLUMN_DISPLAY_NAME,
            )

        val rootName =
            context.contentResolver.query(rootUri, projection, null, null, null)?.use { cursor ->
                if (cursor.moveToFirst()) {
                    val nameIndex =
                        cursor.getColumnIndex(DocumentsContract.Document.COLUMN_DISPLAY_NAME)
                    if (nameIndex >= 0 && !cursor.isNull(nameIndex)) {
                        cursor.getString(nameIndex)
                    } else {
                        documentId
                    }
                } else {
                    documentId
                }
            } ?: documentId

        var count = 0
        var truncated = false
        context.contentResolver.query(childrenUri, projection, null, null, null)?.use { cursor ->
            while (cursor.moveToNext()) {
                if (count == maxEntries) {
                    truncated = true
                    break
                }
                count += 1
            }
        }

        return SafTreeSnapshot(
            rootName = rootName,
            entries = count,
            truncated = truncated,
        )
    }
}
