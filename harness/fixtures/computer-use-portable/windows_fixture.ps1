try {
    Add-Type -AssemblyName System.Windows.Forms
    Add-Type -AssemblyName System.Drawing

    $form = New-Object System.Windows.Forms.Form
    $form.Text = "Clark Computer Use QA"
    $form.StartPosition = "CenterScreen"
    $form.ClientSize = New-Object System.Drawing.Size(560, 360)
    $form.TopMost = $true

    $button = New-Object System.Windows.Forms.Button
    $button.Text = "Click to verify Clark Computer Use"
    $button.Dock = [System.Windows.Forms.DockStyle]::Fill
    $button.Font = New-Object System.Drawing.Font("Segoe UI", 18)
    $button.Add_Click({
        $button.Text = "Clicked by Clark Computer Use"
        $button.BackColor = [System.Drawing.Color]::LightGreen
    })

    $form.Controls.Add($button)
    [System.Windows.Forms.Application]::Run($form)
} catch {
    $_ | Out-String | Out-File -Encoding utf8 C:\Users\Public\windows_fixture_error.txt
    exit 1
}
