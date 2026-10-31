package org.andromeda.link.control

import android.content.Context
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.core.content.edit
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import org.andromeda.link.R
import org.andromeda.link.ui.components.Label
import org.andromeda.link.ui.theme.LinkTheme
import org.andromeda.link.ui.theme.Radius
import org.andromeda.link.ui.theme.Space

/**
 * Keys a phone keyboard lacks, above it: modifiers that stay held until the next key, Esc and Tab, and a row of the
 * user's own shortcuts. A long press removes one of those.
 */
@Composable
internal fun ShortcutBar(
    viewModel: RemoteViewModel,
    modifier: Modifier = Modifier,
) {
    val context = LocalContext.current
    val held by viewModel.held.collectAsStateWithLifecycle()
    var saved by remember { mutableStateOf(savedShortcuts(context)) }
    var adding by rememberSaveable { mutableStateOf(false) }
    val save = { list: List<String> ->
        saved = list
        prefs(context).edit { putString(KEY, list.joinToString("\n")) }
    }
    Column(modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(Space.s8)) {
        Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(Space.s6)) {
            for (name in listOf("super", "ctrl", "alt", "shift")) {
                Chip(label(name), on = name in held) { viewModel.hold(name) }
            }
            Chip("Esc") { viewModel.shortcut("escape") }
            Chip("Tab") { viewModel.shortcut("tab") }
        }
        Row(Modifier.horizontalScroll(rememberScrollState()), horizontalArrangement = Arrangement.spacedBy(Space.s6)) {
            for (shortcut in saved) {
                Chip(label(shortcut), onLongClick = { save(saved - shortcut) }) { viewModel.shortcut(shortcut) }
            }
            if (adding) {
                AddField { typed ->
                    adding = false
                    val shortcut = typed.trim().lowercase().replace(" ", "")
                    if (shortcut.isNotEmpty() && shortcut !in saved) save(saved + shortcut)
                }
            } else {
                Chip(stringResource(R.string.shortcut_add)) { adding = true }
            }
        }
    }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun Chip(
    text: String,
    on: Boolean = false,
    onLongClick: (() -> Unit)? = null,
    onClick: () -> Unit,
) {
    val colors = LinkTheme.colors
    Label(
        text,
        LinkTheme.type.titleSmall,
        if (on) colors.onAccent else colors.textPrimary,
        Modifier
            .clip(Radius.pill)
            .background(if (on) colors.accent else colors.surfaceSecondary)
            .combinedClickable(role = Role.Button, onLongClick = onLongClick, onClick = onClick)
            .padding(horizontal = Space.s12, vertical = Space.s8),
    )
}

@Composable
private fun AddField(onDone: (String) -> Unit) {
    var text by rememberSaveable { mutableStateOf("") }
    BasicTextField(
        value = text,
        onValueChange = { text = it },
        modifier =
            Modifier
                .clip(Radius.pill)
                .background(LinkTheme.colors.surfaceSecondary)
                .padding(horizontal = Space.s12, vertical = Space.s8),
        textStyle = LinkTheme.type.titleSmall.copy(color = LinkTheme.colors.textPrimary),
        cursorBrush = SolidColor(LinkTheme.colors.accent),
        singleLine = true,
        keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.None, imeAction = ImeAction.Done),
        keyboardActions = KeyboardActions(onDone = { onDone(text) }),
        decorationBox = { field ->
            if (text.isEmpty()) Label(stringResource(R.string.shortcut_hint), LinkTheme.type.titleSmall, LinkTheme.colors.textTertiary)
            field()
        },
    )
}

/** `super+t` as `Super+T`. */
private fun label(shortcut: String) = shortcut.split('+').joinToString("+") { part -> part.replaceFirstChar { it.uppercase() } }

private fun prefs(context: Context) = context.getSharedPreferences("remote", Context.MODE_PRIVATE)

private fun savedShortcuts(context: Context): List<String> =
    prefs(context).getString(KEY, null)?.split("\n")?.filter { it.isNotEmpty() } ?: DEFAULT

private const val KEY = "shortcuts"
private val DEFAULT = listOf("alt+tab", "super+t", "ctrl+c", "ctrl+v")
