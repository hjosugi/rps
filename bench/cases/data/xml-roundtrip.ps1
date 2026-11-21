# Build, serialise and query an XmlDocument with 2,000 elements.
$xml = [xml]'<root></root>'
for ($i = 1; $i -le 2000; $i++) {
    $item = $xml.CreateElement('item')
    $item.SetAttribute('id', [string]$i)
    $item.InnerText = "value $($i % 100)"
    [void]$xml.DocumentElement.AppendChild($item)
}
$xml.OuterXml.Length
$xml.SelectNodes('//item').Count
$xml.SelectSingleNode('//item[@id="1000"]').InnerText
