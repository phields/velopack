﻿using System.CommandLine;
using System.CommandLine.Parsing;
using Squirrel.CommandLine.Deployment;
using Xunit;

namespace Squirrel.CommandLine.Tests.Deployment
{
    public class HttpDownloadCommandTests : BaseCommandTests<HttpDownloadCommand>
    {
        [Fact]
        public void Url_WithUrl_ParsesValue()
        {
            var command = new HttpDownloadCommand();

            ParseResult parseResult = command.Parse($"--url \"http://clowd.squirrel.com\"");

            Assert.Empty(parseResult.Errors);
            Assert.Equal("http://clowd.squirrel.com/", parseResult.GetValueForOption(command.Url)?.AbsoluteUri);
        }

        [Fact]
        public void Url_WithNonHttpValue_ShowsError()
        {
            var command = new HttpDownloadCommand();

            ParseResult parseResult = command.Parse($"--url \"file://clowd.squirrel.com\"");

            Assert.Equal(1, parseResult.Errors.Count);
            Assert.Equal(command.Url, parseResult.Errors[0].SymbolResult?.Symbol);
            Assert.StartsWith("--url must contain a Uri with one of the following schems: http, https.", parseResult.Errors[0].Message);
        }

        [Fact]
        public void Url_WithRelativeUrl_ShowsError()
        {
            var command = new HttpDownloadCommand();

            ParseResult parseResult = command.Parse($"--url \"clowd.squirrel.com\"");

            Assert.Equal(1, parseResult.Errors.Count);
            Assert.Equal(command.Url, parseResult.Errors[0].SymbolResult?.Symbol);
            Assert.StartsWith("--url must contain an absolute Uri.", parseResult.Errors[0].Message);
        }

        protected override string GetRequiredDefaultOptions()
        {
            return $"--url \"https://clowd.squirrel.com\" ";
        }
    }
}
