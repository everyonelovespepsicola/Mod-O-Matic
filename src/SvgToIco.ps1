Add-Type -AssemblyName PresentationFramework

[xml]$xaml = @"
<Window xmlns="http://schemas.microsoft.com/winfx/2006/xaml/presentation"
        xmlns:x="http://schemas.microsoft.com/winfx/2006/xaml"
        Title="Image to ICO Converter" Height="250" Width="400" WindowStartupLocation="CenterScreen">
    <Grid>
        <Border x:Name="DropZone" BorderBrush="SteelBlue" BorderThickness="2" CornerRadius="10" Margin="20" Background="#F0F8FF" AllowDrop="True">
            <TextBlock x:Name="StatusText" Text="Drop an image file here to convert to ICO" HorizontalAlignment="Center" VerticalAlignment="Center" FontSize="16" Foreground="#4682B4" TextWrapping="Wrap" TextAlignment="Center"/>
        </Border>
    </Grid>
</Window>
"@

$reader = (New-Object System.Xml.XmlNodeReader $xaml)
$window = [Windows.Markup.XamlReader]::Load($reader)

$dropZone = $window.FindName("DropZone")
$statusText = $window.FindName("StatusText")

$dropZone.add_DragOver({
        $_.Effects = [System.Windows.DragDropEffects]::Copy
        $_.Handled = $true
    })

$dropZone.add_Drop({
        $_.Handled = $true
        $files = $_.Data.GetData([System.Windows.DataFormats]::FileDrop)
        if ($files.Count -eq 0) { return }

        $filePath = $files[0]
        $ext = [System.IO.Path]::GetExtension($filePath).ToLower()
        $validExts = @('.svg', '.png', '.jpg', '.jpeg', '.gif', '.bmp', '.webp')

        if ($validExts -notcontains $ext) {
            $statusText.Text = "Please drop a valid image file (SVG, PNG, JPG, etc.)."
            $statusText.Foreground = "Red"
            return
        }

        $statusText.Text = "Processing: $([System.IO.Path]::GetFileName($filePath))..."
        $statusText.Foreground = "Black"

        # Force a quick UI refresh to show the "Processing..." text before the script blocks
        $window.Dispatcher.Invoke([Action] {}, [System.Windows.Threading.DispatcherPriority]::Background)

        try {
            if (!(Get-Command "magick" -ErrorAction SilentlyContinue)) { throw "ImageMagick is not installed or not in PATH." }

            $icoPath = [System.IO.Path]::ChangeExtension($filePath, ".ico")
            $tempFiles = @()

            if ($ext -eq ".svg") {
                if (!(Get-Command "inkscape" -ErrorAction SilentlyContinue)) { throw "Inkscape is not installed or not in PATH." }

                # Standard multi-resolution sizes for Windows ICO files
                $sizes = @(16, 32, 48, 64, 128, 256)

                # 1. Rasterize cleanly with Inkscape
                foreach ($size in $sizes) {
                    $tempPng = [System.IO.Path]::Combine([System.IO.Path]::GetTempPath(), "$([guid]::NewGuid())_$size.png")
                    $tempFiles += $tempPng

                    # Inkscape 1.0+ command line syntax
                    $inkscapeArgs = "-w $size -h $size -o `"$tempPng`" `"$filePath`""
                    $p = Start-Process -FilePath "inkscape" -ArgumentList $inkscapeArgs -Wait -NoNewWindow -PassThru
                    if ($p.ExitCode -ne 0) { throw "Inkscape failed to process $size x $size." }
                }

                # 2. Combine and pack with ImageMagick
                $magickArgsStr = ""
                foreach ($file in $tempFiles) { $magickArgsStr += "`"$file`" " }
                $magickArgsStr += "`"$icoPath`""
                $p = Start-Process -FilePath "magick" -ArgumentList $magickArgsStr -Wait -NoNewWindow -PassThru
                if ($p.ExitCode -ne 0) { throw "ImageMagick failed to pack ICO." }
            }
            else {
                # Let ImageMagick handle raster images directly by automatically building the internal layers
                $magickArgsStr = "`"$filePath`" -define icon:auto-resize=256,128,64,48,32,16 `"$icoPath`""
                $p = Start-Process -FilePath "magick" -ArgumentList $magickArgsStr -Wait -NoNewWindow -PassThru
                if ($p.ExitCode -ne 0) { throw "ImageMagick failed to create ICO." }
            }

            $statusText.Text = "Success! Created:`n$icoPath"
            $statusText.Foreground = "Green"
        }
        catch {
            $statusText.Text = "Error: $_"
            $statusText.Foreground = "Red"
        }
        finally {
            # Cleanup temp PNG layers
            foreach ($file in $tempFiles) {
                if (Test-Path $file) { Remove-Item $file -Force }
            }
        }
    })

# Requires STA mode for drag and drop to work correctly in PowerShell
[System.Threading.Thread]::CurrentThread.SetApartmentState([System.Threading.ApartmentState]::STA)
$window.ShowDialog() | Out-Null
